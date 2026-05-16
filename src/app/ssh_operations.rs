use std::sync::mpsc;

use super::DirigentApp;
use crate::settings::SshAuthKind;
use crate::ssh::{SshAuthMethod, SshRequest, SshServerConfig};

impl DirigentApp {
    pub(super) fn ssh_connect(&mut self, server_idx: usize) {
        if self.ssh_connecting {
            return;
        }
        let server = match self.settings.ssh_servers.get(server_idx) {
            Some(s) => s,
            None => return,
        };
        let config = SshServerConfig {
            name: server.name.clone(),
            host: server.host.clone(),
            port: server.port,
            username: server.username.clone(),
            auth_method: match &server.auth_kind {
                SshAuthKind::Agent => SshAuthMethod::Agent,
                SshAuthKind::KeyFile => SshAuthMethod::KeyFile {
                    path: crate::app::util::expand_tilde(&server.key_path),
                },
                SshAuthKind::Password => SshAuthMethod::Password {
                    password: server.password.clone(),
                },
            },
            remote_path: server.remote_path.clone(),
        };
        self.ssh_connecting = true;
        let rx = crate::ssh::spawn_ssh_worker(config, self.egui_ctx.clone());
        self.ssh_connect_rx = Some(rx);
    }

    pub(super) fn ssh_disconnect(&mut self) {
        if let Some(worker) = self.ssh_worker.take() {
            worker.send(SshRequest::Disconnect);
        }
        self.ssh_remote_entries.clear();
        self.ssh_remote_path.clear();
        self.ssh_expanded_dirs.clear();
        self.ssh_listing = false;
        self.ssh_reading_file = None;
        self.show_ssh_panel = false;
        self.set_status_message("SSH disconnected".into());
    }

    pub(super) fn ssh_list_dir(&mut self, path: &str) {
        if self.ssh_listing {
            return;
        }
        if let Some(worker) = self.ssh_worker.as_ref() {
            worker.send(SshRequest::ListDir(path.to_string()));
        } else {
            return;
        }
        self.ssh_listing = true;
        self.set_status_message("Listing remote directory\u{2026}".into());
    }

    pub(super) fn ssh_read_file(&mut self, path: &str) {
        if self.ssh_reading_file.is_some() {
            return;
        }
        if let Some(worker) = self.ssh_worker.as_ref() {
            worker.send(SshRequest::ReadFile(path.to_string()));
        } else {
            return;
        }
        self.ssh_reading_file = Some(path.to_string());
    }

    pub(super) fn process_ssh_connect_result(&mut self) {
        let rx = match self.ssh_connect_rx.as_ref() {
            Some(rx) => rx,
            None => return,
        };
        let result = match rx.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.ssh_connecting = false;
                self.ssh_connect_rx = None;
                self.set_status_message("SSH connection thread died".into());
                return;
            }
        };
        self.ssh_connecting = false;
        self.ssh_connect_rx = None;
        match result {
            Ok(handle) => {
                let remote_path = handle.config.remote_path.clone();
                let name = handle.config.name.clone();
                self.ssh_worker = Some(handle);
                self.show_ssh_panel = true;
                self.set_status_message(format!("Connected to '{}'", name));
                self.ssh_list_dir(&remote_path);
            }
            Err(e) => {
                self.set_status_message(format!("SSH connection failed: {}", e));
            }
        }
    }

    pub(super) fn process_ssh_responses(&mut self) {
        use crate::ssh::SshResponse;

        let worker = match self.ssh_worker.as_ref() {
            Some(w) => w,
            None => return,
        };

        let mut responses = Vec::new();
        while let Ok(resp) = worker.rx.try_recv() {
            responses.push(resp);
        }

        for resp in responses {
            match resp {
                SshResponse::ListDir(Ok((path, entries))) => {
                    self.ssh_listing = false;
                    self.ssh_remote_path = path;
                    self.ssh_remote_entries = entries;
                    self.set_status_message(String::new());
                }
                SshResponse::ListDir(Err(e)) => {
                    self.ssh_listing = false;
                    self.set_status_message(format!("SSH list dir failed: {}", e));
                }
                SshResponse::ReadFile(Ok((path, contents))) => {
                    self.ssh_reading_file = None;
                    self.handle_ssh_file_contents(&path, &contents);
                }
                SshResponse::ReadFile(Err(e)) => {
                    let path = self.ssh_reading_file.take().unwrap_or_default();
                    self.set_status_message(format!(
                        "Failed to read remote file '{}': {}",
                        path, e
                    ));
                }
                SshResponse::Disconnected => {
                    self.ssh_worker = None;
                    self.ssh_remote_entries.clear();
                    self.ssh_remote_path.clear();
                    self.ssh_expanded_dirs.clear();
                    self.ssh_listing = false;
                    self.ssh_reading_file = None;
                    self.show_ssh_panel = false;
                }
            }
        }
    }

    fn handle_ssh_file_contents(&mut self, remote_path: &str, contents: &str) {
        let tab_path = std::path::PathBuf::from(format!("ssh://{}", remote_path));
        if let Some(idx) = self
            .viewer
            .tabs
            .iter()
            .position(|t| t.file_path == tab_path)
        {
            self.viewer.active_tab = Some(idx);
            return;
        }

        let file_name = remote_path
            .rsplit('/')
            .next()
            .unwrap_or(remote_path)
            .to_string();

        let lines: Vec<String> = contents.lines().map(|l| l.to_string()).collect();

        let ext = file_name.rsplit('.').next().unwrap_or("").to_lowercase();
        let is_markdown = ext == "md" || ext == "mdx";

        let markdown_blocks = if is_markdown {
            Some(crate::app::markdown_parser::parse_markdown(
                &lines.join("\n"),
            ))
        } else {
            None
        };

        use crate::app::types::TabState;
        let tab = TabState {
            file_path: tab_path,
            content: lines,
            selection_start: None,
            selection_end: None,
            cue_input: String::new(),
            cue_images: Vec::new(),
            markdown_blocks,
            markdown_rendered: is_markdown,
            scroll_offset: 0.0,
            symbols: Vec::new(),
            image_data: None,
            image_texture: None,
            image_zoom: 1.0,
            last_mtime: None,
        };

        if self.viewer.tabs.len() >= 20 {
            let close_idx = if self.viewer.active_tab == Some(0) {
                1
            } else {
                0
            };
            self.viewer.close_tab(close_idx);
        }
        self.viewer.tabs.push(tab);
        self.viewer.active_tab = Some(self.viewer.tabs.len() - 1);
        self.dismiss_central_overlays();
    }
}
