use std::sync::mpsc;

use super::DirigentApp;
use crate::settings::SshAuthKind;
use crate::ssh::{SshAuthMethod, SshConnection, SshServerConfig};

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
        let (tx, rx) = mpsc::channel();
        let ctx = self.egui_ctx.clone();
        std::thread::spawn(move || {
            let result = SshConnection::connect(&config);
            let _ = tx.send(result);
            if let Some(c) = ctx.get() {
                c.request_repaint();
            }
        });
        self.ssh_connect_rx = Some(rx);
    }

    pub(super) fn ssh_disconnect(&mut self) {
        if let Some(conn) = self.ssh_connection.take() {
            conn.disconnect();
        }
        self.ssh_remote_entries.clear();
        self.ssh_remote_path.clear();
        self.ssh_expanded_dirs.clear();
        self.show_ssh_panel = false;
        self.set_status_message("SSH disconnected".into());
    }

    pub(super) fn ssh_list_dir(&mut self, path: &str) {
        let Some(ref conn) = self.ssh_connection else {
            return;
        };
        // We can't move the connection to a thread (it's not Send for ssh2),
        // so perform SFTP listing on the main thread. For a better UX this
        // could be refactored to use a dedicated SSH thread with a channel,
        // but for now synchronous SFTP is acceptable.
        match conn.list_dir(path) {
            Ok(entries) => {
                self.ssh_remote_path = path.to_string();
                self.ssh_remote_entries = entries;
            }
            Err(e) => {
                self.set_status_message(format!("SSH list dir failed: {}", e));
            }
        }
    }

    pub(super) fn ssh_read_file(&self, path: &str) -> Option<String> {
        let conn = self.ssh_connection.as_ref()?;
        match conn.read_file(path) {
            Ok(contents) => Some(contents),
            Err(e) => {
                eprintln!("SSH read file failed: {}", e);
                None
            }
        }
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
            Ok(conn) => {
                let remote_path = conn.config.remote_path.clone();
                let name = conn.config.name.clone();
                self.ssh_connection = Some(conn);
                self.show_ssh_panel = true;
                self.set_status_message(format!("Connected to '{}'", name));
                self.ssh_list_dir(&remote_path);
            }
            Err(e) => {
                self.set_status_message(format!("SSH connection failed: {}", e));
            }
        }
    }
}
