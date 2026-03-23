use std::collections::HashMap;
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
                    && self.sources.last_poll.get(&s.name).map_or(true, |last| {
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
            let items = match source.kind {
                SourceKind::GitHubIssues => {
                    let label_filter = if source.filter.is_empty() {
                        None
                    } else {
                        Some(source.filter.as_str())
                    };
                    sources::fetch_github_issues(&project_root, label_filter, None, &source.label)
                        .unwrap_or_else(|e| {
                            let _ = error_tx.send(format!("Source '{}': {}", source.name, e));
                            Vec::new()
                        })
                }
                SourceKind::Slack => {
                    sources::fetch_slack_messages(&source.token, &source.channel, &source.label)
                        .unwrap_or_else(|e| {
                            let _ = error_tx.send(format!("Source '{}': {}", source.name, e));
                            Vec::new()
                        })
                }
                SourceKind::SonarQube => sources::fetch_sonarqube_issues(
                    &project_root,
                    &source.host_url,
                    &source.project_key,
                    &source.token,
                    &source.label,
                )
                .unwrap_or_else(|e| {
                    let _ = error_tx.send(format!("Source '{}': {}", source.name, e));
                    Vec::new()
                }),
                SourceKind::Custom | SourceKind::Notion | SourceKind::Mcp => {
                    if source.command.is_empty() {
                        Vec::new()
                    } else {
                        sources::fetch_custom_command(&project_root, &source.command, &source.label)
                            .unwrap_or_else(|e| {
                                let _ = error_tx.send(format!("Source '{}': {}", source.name, e));
                                Vec::new()
                            })
                    }
                }
            };
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
                Ok(true) => continue,
                Ok(false) => {}
                Err(_) => continue,
            }
            if self
                .db
                .insert_cue_from_source(&item.text, &item.source_label, &item.external_id)
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
