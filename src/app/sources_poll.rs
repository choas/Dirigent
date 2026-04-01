use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Instant;

use crate::settings::{self, SourceKind};
use crate::sources::{self, SourceItem};

use super::tasks::TaskHandle;
use super::DirigentApp;

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
        let project_root = self.project_root.clone();
        let source_tx = self.sources.tx.clone();
        let error_tx = self.sources.error_tx.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_thread = Arc::clone(&cancel);

        let join_handle = std::thread::spawn(move || {
            if cancel_thread.load(Ordering::Relaxed) {
                return;
            }
            let items = fetch_source_items(&source, &project_root, &error_tx);
            if cancel_thread.load(Ordering::Relaxed) {
                return;
            }
            for item in items {
                let _ = source_tx.send(item);
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
            self.trigger_source_fetch_config(source);
            self.set_status_message(msg);
        }
    }

    pub(super) fn process_source_results(&mut self) {
        // Surface any source fetch errors to the UI
        if let Ok(err_msg) = self.sources.error_rx.try_recv() {
            self.set_status_message(err_msg);
        }

        let items: Vec<SourceItem> = self.sources.rx.try_iter().collect();

        if items.is_empty() {
            return;
        }

        let mut new_count = 0;
        for item in items {
            match self.db.cue_exists_by_source_ref(&item.external_id) {
                Ok(true) => {
                    // Backfill source_id on migrated rows that have it NULL.
                    if !item.source_id.is_empty() {
                        let _ = self.db.backfill_source_id(
                            &item.external_id,
                            &item.source_id,
                            &item.source_label,
                        );
                    }
                    continue;
                }
                Ok(false) => {}
                Err(_) => continue,
            }
            if self
                .db
                .insert_cue_from_source(
                    &item.text,
                    &item.source_label,
                    &item.source_id,
                    &item.external_id,
                    "",
                    0,
                )
                .is_ok()
            {
                new_count += 1;
            }
        }
        if new_count > 0 {
            self.reload_cues();
            self.set_status_message(format!("{} new cue(s) from sources", new_count));
        }
    }
}

fn fetch_source_items(
    source: &settings::SourceConfig,
    project_root: &Path,
    error_tx: &mpsc::Sender<String>,
) -> Vec<SourceItem> {
    let err = |e: crate::error::DirigentError| {
        let _ = error_tx.send(format!("Source '{}': {}", source.name, e));
        Vec::new()
    };
    let mut items = match source.kind {
        SourceKind::GitHubIssues => {
            let label_filter = (!source.filter.is_empty()).then(|| source.filter.as_str());
            sources::fetch_github_issues(project_root, label_filter, None, &source.label)
                .unwrap_or_else(err)
        }
        SourceKind::Slack => {
            let token = sources::resolve_source_token(source, project_root);
            sources::fetch_slack_messages(&token, &source.channel, &source.label)
                .unwrap_or_else(err)
        }
        SourceKind::SonarQube => {
            let host = if source.host_url.is_empty() {
                "http://localhost:9000"
            } else {
                &source.host_url
            };
            let token = sources::resolve_source_token(source, project_root);
            sources::fetch_sonarqube_issues(host, &source.project_key, &token, &source.label)
                .unwrap_or_else(err)
        }
        SourceKind::Trello => {
            let api_key = if !source.api_key.is_empty() {
                source.api_key.clone()
            } else {
                std::env::var("TRELLO_API_KEY")
                    .ok()
                    .or_else(|| sources::load_env_var(project_root, "TRELLO_API_KEY"))
                    .unwrap_or_default()
            };
            let token = sources::resolve_source_token(source, project_root);
            let list_filter = if source.filter.is_empty() {
                None
            } else {
                Some(source.filter.as_str())
            };
            sources::fetch_trello_cards(
                &api_key,
                &token,
                &source.project_key,
                list_filter,
                &source.label,
            )
            .unwrap_or_else(err)
        }
        SourceKind::Asana => {
            let token = sources::resolve_source_token(source, project_root);
            sources::fetch_asana_tasks(&token, &source.project_key, &source.label)
                .unwrap_or_else(err)
        }
        SourceKind::Notion => {
            let token = sources::resolve_source_token(source, project_root);
            let inbox_status = if source.filter.is_empty() {
                None
            } else {
                Some(source.filter.as_str())
            };
            sources::fetch_notion_tasks(
                &token,
                &source.project_key,
                &source.notion_page_type,
                inbox_status,
                &source.notion_done_value,
                &source.notion_status_property,
                &source.label,
            )
            .unwrap_or_else(err)
        }
        SourceKind::Custom | SourceKind::Mcp => {
            if source.command.is_empty() {
                Vec::new()
            } else {
                sources::fetch_custom_command(
                    project_root,
                    &source.command,
                    &source.label,
                    source.id.as_deref().unwrap_or(""),
                )
                .unwrap_or_else(err)
            }
        }
    };
    // Stamp the stable source identifier on every item.
    for item in &mut items {
        item.source_id = source.id.clone().unwrap_or_default();
    }
    items
}
