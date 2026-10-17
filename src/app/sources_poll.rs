use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Instant;

use crate::db::CueStatus;
use crate::settings::{self, SourceKind};
use crate::sources::{self, SourceItem};

use super::tasks::TaskHandle;
use super::{CueAction, DirigentApp};

/// State for external cue source polling.
pub(crate) struct SourceState {
    tx: mpsc::Sender<SourceItem>,
    rx: mpsc::Receiver<SourceItem>,
    error_tx: mpsc::Sender<String>,
    error_rx: mpsc::Receiver<String>,
    pub(super) last_poll: HashMap<String, Instant>,
    pub(super) filter: Option<String>,
}

impl SourceState {
    pub(super) fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let (error_tx, error_rx) = mpsc::channel();
        SourceState {
            tx,
            rx,
            error_tx,
            error_rx,
            last_poll: HashMap::new(),
            filter: None,
        }
    }
}

impl DirigentApp {
    pub(super) fn poll_sources(&mut self) {
        // Collect sources to poll first to avoid borrow conflict with &mut self.
        let to_poll: Vec<settings::SourceConfig> = self
            .settings
            .sources
            .iter()
            .filter(|s| {
                s.enabled
                    && s.poll_interval_secs > 0
                    && self.sources.last_poll.get(&s.name).is_none_or(|last| {
                        last.elapsed() >= std::time::Duration::from_secs(s.poll_interval_secs)
                    })
            })
            .cloned()
            .collect();

        for source in to_poll {
            self.sources
                .last_poll
                .insert(source.name.clone(), Instant::now());
            self.trigger_source_fetch_config(source);
        }
    }

    fn trigger_source_fetch_config(&mut self, source: settings::SourceConfig) {
        self.trigger_source_fetch_config_inner(source, None);
    }

    /// Spawn a background fetch for `source`. When `status_tx` is provided
    /// (manual "Fetch Now"), the outcome is also reported there as
    /// `Ok(item_count)` / `Err(message)` so the settings dialog can show it inline.
    fn trigger_source_fetch_config_inner(
        &mut self,
        source: settings::SourceConfig,
        status_tx: Option<mpsc::Sender<Result<usize, String>>>,
    ) {
        let project_root = self.project_root.clone();
        let source_tx = self.sources.tx.clone();
        let error_tx = self.sources.error_tx.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_thread = Arc::clone(&cancel);
        let ctx = Arc::clone(&self.egui_ctx);

        let source_name = source.name.clone();
        let join_handle = std::thread::spawn(move || {
            if cancel_thread.load(Ordering::Relaxed) {
                return;
            }
            let result = fetch_source_items(&source, &project_root);
            if cancel_thread.load(Ordering::Relaxed) {
                return;
            }
            publish_source_fetch_result(result, &source_name, &source_tx, &error_tx, status_tx);
            if let Some(c) = ctx.get() {
                c.request_repaint();
            }
        });

        self.task_handles.push(TaskHandle {
            join_handle,
            cancel,
            cue_id: None,
            exec_id: None,
        });
    }

    /// Trigger a manual fetch for a source by its index in settings.
    pub(super) fn trigger_source_fetch(&mut self, idx: usize) {
        if let Some(source) = self.settings.sources.get(idx).cloned() {
            self.sources
                .last_poll
                .insert(source.name.clone(), Instant::now());
            let msg = format!("Fetching from \"{}\"...", source.name);
            let (tx, rx) = mpsc::channel();
            self.source_fetch_status.remove(&idx);
            self.source_fetch_rx = Some((idx, rx));
            self.trigger_source_fetch_config_inner(source, Some(tx));
            self.set_status_message(msg);
        }
    }

    /// Drain the result of a manual "Fetch Now" into `source_fetch_status` so the
    /// settings dialog can render it inline. Called from the update loop.
    pub(super) fn process_source_fetch_status(&mut self) {
        let (idx, result) = match &self.source_fetch_rx {
            Some((idx, rx)) => match rx.try_recv() {
                Ok(result) => (*idx, result),
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => {
                    let idx = *idx;
                    self.source_fetch_rx = None;
                    self.source_fetch_status
                        .insert(idx, Err("fetch failed unexpectedly".to_string()));
                    return;
                }
            },
            None => return,
        };
        self.source_fetch_rx = None;
        self.source_fetch_status.insert(idx, result);
    }

    pub(super) fn process_source_results(&mut self) {
        self.process_source_errors();

        let items: Vec<SourceItem> = self.sources.rx.try_iter().collect();

        if items.is_empty() {
            return;
        }

        let total_received = items.len();
        // Runnable cues execute through the AI CLI without further
        // confirmation, and `allow_runnable` comes from the same untrusted
        // `.Dirigent/settings.json` as the sources themselves. Honor the
        // `{runnable}` marker only when the repository's settings are covered
        // by an explicit user approval (defense in depth alongside the
        // load-time trust gate).
        let repo_trusted = crate::settings::repo_settings_trusted(&self.project_root);
        let mut new_count = 0;
        let mut failed_count = 0;
        let mut auto_run_ids: Vec<i64> = Vec::new();
        for item in items {
            let Some((text, runnable)) = self.prepare_source_item(&item, repo_trusted) else {
                continue;
            };
            match self.db.insert_cue_from_source(
                &text,
                &item.source_label,
                &item.source_id,
                &item.external_id,
                &item.file_path,
                item.line_number,
            ) {
                Ok(cue_id) => {
                    new_count += 1;
                    if runnable {
                        auto_run_ids.push(cue_id);
                    }
                }
                Err(e) => {
                    log::error!("[db] create cue from source failed: {:#}", e);
                    failed_count += 1;
                }
            }
        }
        self.finish_source_import(new_count, failed_count, auto_run_ids, total_received);
    }

    fn process_source_errors(&mut self) {
        let errors: Vec<String> = self.sources.error_rx.try_iter().collect();
        for msg in errors {
            self.set_status_message(msg);
        }
    }

    fn prepare_source_item(
        &mut self,
        item: &SourceItem,
        repo_trusted: bool,
    ) -> Option<(String, bool)> {
        match self.db.cue_exists_by_source_ref(&item.external_id) {
            Ok(true) => {
                self.refresh_existing_source_cue(item);
                return None;
            }
            Ok(false) => {}
            Err(e) => {
                log::error!("[db] check for existing source cue failed: {:#}", e);
                return None;
            }
        }
        let runnable =
            repo_trusted && self.source_allows_runnable(item) && has_runnable_marker(&item.text);
        let text = if runnable {
            strip_runnable_marker(&item.text)
        } else {
            item.text.clone()
        };
        (!text.trim().is_empty()).then_some((text, runnable))
    }

    fn refresh_existing_source_cue(&self, item: &SourceItem) {
        self.db_log(
            "refresh source cue text",
            self.db
                .update_cue_text_by_source_ref(&item.external_id, &item.text),
        );
        if !item.source_id.is_empty() {
            self.db_log(
                "backfill source id",
                self.db
                    .backfill_source_id(&item.external_id, &item.source_id, &item.source_label),
            );
        }
    }

    fn source_allows_runnable(&self, item: &SourceItem) -> bool {
        self.settings
            .sources
            .iter()
            .find(|source| source.id.as_deref() == Some(&item.source_id))
            .is_some_and(|source| source.allow_runnable)
    }

    fn finish_source_import(
        &mut self,
        new_count: usize,
        failed_count: usize,
        auto_run_ids: Vec<i64>,
        total_received: usize,
    ) {
        // A failed insert means an item was fetched but never persisted, so the
        // user would otherwise just see it missing from the Inbox.
        let failed_suffix = if failed_count > 0 {
            format!(" — {} could not be saved (database error)", failed_count)
        } else {
            String::new()
        };
        if new_count == 0 {
            self.set_status_message(format!(
                "Source fetch complete: {} item(s) received, no new items{}",
                total_received, failed_suffix
            ));
            return;
        }
        self.reload_cues();
        let auto_count = auto_run_ids.len();
        for cue_id in auto_run_ids {
            self.process_cue_action(cue_id, CueAction::MoveTo(CueStatus::Ready));
        }
        if auto_count > 0 {
            self.set_status_message(format!(
                "{} new cue(s) from sources ({} auto-running){}",
                new_count, auto_count, failed_suffix
            ));
        } else {
            self.set_status_message(format!(
                "{} new cue(s) from sources{}",
                new_count, failed_suffix
            ));
        }
    }
}

fn publish_source_fetch_result(
    result: crate::error::Result<Vec<SourceItem>>,
    source_name: &str,
    source_tx: &mpsc::Sender<SourceItem>,
    error_tx: &mpsc::Sender<String>,
    status_tx: Option<mpsc::Sender<Result<usize, String>>>,
) {
    match &result {
        Ok(items) if items.is_empty() => {
            let _ = error_tx.send(format!("Source \"{}\": fetched 0 items", source_name));
        }
        Ok(items) => {
            for item in items.iter().cloned() {
                let _ = source_tx.send(item);
            }
        }
        Err(e) => {
            let _ = error_tx.send(format!("Source \"{}\": {}", source_name, e));
        }
    }
    if let Some(tx) = status_tx {
        let _ = tx.send(result.map(|items| items.len()).map_err(|e| e.to_string()));
    }
}

const RUNNABLE_TOKEN: &str = "{runnable}";

/// Check whether any line in `text` starts with `{runnable}` (after optional
/// leading whitespace).  Quoted or mid-line mentions do not match.
fn has_runnable_marker(text: &str) -> bool {
    text.lines()
        .any(|line| line.trim_start().starts_with(RUNNABLE_TOKEN))
}

/// Remove the first line-start `{runnable}` marker and clean up whitespace.
fn strip_runnable_marker(text: &str) -> String {
    let mut found = false;
    let lines: Vec<&str> = text
        .lines()
        .filter_map(|line| {
            if !found {
                if let Some(rest) = line.trim_start().strip_prefix(RUNNABLE_TOKEN) {
                    found = true;
                    let rest = rest.trim();
                    return if rest.is_empty() { None } else { Some(rest) };
                }
            }
            Some(line)
        })
        .collect();
    lines.join("\n").trim().to_string()
}

fn fetch_source_items(
    source: &settings::SourceConfig,
    project_root: &Path,
) -> crate::error::Result<Vec<SourceItem>> {
    let mut items = fetch_by_kind(source, project_root)?;
    for item in &mut items {
        item.source_id = source.id.clone().unwrap_or_default();
    }
    Ok(items)
}

fn non_empty_or<'a>(value: &'a str, default: &'a str) -> &'a str {
    if value.is_empty() {
        default
    } else {
        value
    }
}

fn resolve_trello_api_key(source: &settings::SourceConfig, project_root: &Path) -> String {
    if !source.api_key.is_empty() {
        source.api_key.clone()
    } else {
        std::env::var("TRELLO_API_KEY")
            .ok()
            .or_else(|| sources::load_env_var(project_root, "TRELLO_API_KEY"))
            .unwrap_or_default()
    }
}

fn fetch_custom_source(
    source: &settings::SourceConfig,
    project_root: &Path,
) -> crate::error::Result<Vec<SourceItem>> {
    if source.command.is_empty() {
        Ok(Vec::new())
    } else {
        sources::fetch_custom_command(
            project_root,
            &source.command,
            &source.label,
            source.id.as_deref().unwrap_or(""),
        )
    }
}

fn fetch_by_kind(
    source: &settings::SourceConfig,
    project_root: &Path,
) -> crate::error::Result<Vec<SourceItem>> {
    let token = || sources::resolve_source_token(source, project_root);
    let nonempty_filter = || (!source.filter.is_empty()).then_some(source.filter.as_str());

    match source.kind {
        SourceKind::GitHubIssues => {
            sources::fetch_github_issues(project_root, nonempty_filter(), None, &source.label)
        }
        SourceKind::Slack => {
            sources::fetch_slack_messages(&token(), &source.channel, &source.label)
        }
        SourceKind::SonarQube => {
            let host = non_empty_or(&source.host_url, "http://localhost:9000");
            sources::fetch_sonarqube_issues(host, &source.project_key, &token(), &source.label)
        }
        SourceKind::Trello => {
            let api_key = resolve_trello_api_key(source, project_root);
            sources::fetch_trello_cards(
                &api_key,
                &token(),
                &source.project_key,
                nonempty_filter(),
                &source.label,
            )
        }
        SourceKind::Asana => {
            sources::fetch_asana_tasks(&token(), &source.project_key, &source.label)
        }
        SourceKind::Sentry => sources::fetch_sentry_issues(
            &token(),
            &source.channel,
            &source.project_key,
            &source.host_url,
            &source.label,
        ),
        SourceKind::Notion => sources::fetch_notion_tasks(
            &token(),
            &source.project_key,
            &source.notion_page_type,
            nonempty_filter(),
            &source.notion_done_value,
            &source.notion_status_property,
            &source.label,
        ),
        SourceKind::Custom | SourceKind::Mcp => fetch_custom_source(source, project_root),
    }
}
