use eframe::egui;

use crate::app::{icon, DirigentApp, SPACE_SM, SPACE_XS};
use crate::settings::{SshAuthKind, SshServer};

impl DirigentApp {
    pub(in crate::app) fn render_settings_ssh_section(&mut self, ui: &mut egui::Ui, fs: f32) {
        let summary = format!("{}", self.settings.ssh_servers.len());
        self.ssh_expanded = super::collapsible_section_header(
            ui,
            self.ssh_expanded,
            "SSH Servers",
            &summary,
            fs,
            self.semantic.secondary_text,
            |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("+ Add Server").clicked() {
                        self.settings.ssh_servers.push(SshServer::default());
                    }
                });
            },
        );

        if self.ssh_expanded {
            self.render_settings_ssh_list(ui, fs);
        }
    }

    fn render_settings_ssh_list(&mut self, ui: &mut egui::Ui, fs: f32) {
        ui.add_space(SPACE_SM);

        if self.settings.ssh_servers.is_empty() {
            ui.label(
                egui::RichText::new(
                    "No SSH servers configured. Add a server to browse remote files via SFTP.",
                )
                .italics()
                .color(self.semantic.tertiary_text),
            );
        }

        let mut remove_idx = None;
        let num = self.settings.ssh_servers.len();

        for i in 0..num {
            self.render_settings_ssh_card(ui, i, fs, &mut remove_idx);
            ui.add_space(SPACE_SM);
        }

        if let Some(idx) = remove_idx {
            self.settings.ssh_servers.remove(idx);
        }
    }

    fn render_settings_ssh_card(
        &mut self,
        ui: &mut egui::Ui,
        idx: usize,
        fs: f32,
        remove_idx: &mut Option<usize>,
    ) {
        let server = &self.settings.ssh_servers[idx];
        let header = if server.name.is_empty() {
            format!("Server {}", idx + 1)
        } else {
            server.name.clone()
        };

        egui::CollapsingHeader::new(icon(&header, fs))
            .id_salt(format!("ssh_{}", idx))
            .default_open(server.host.is_empty())
            .show(ui, |ui| {
                egui::Grid::new(format!("ssh_grid_{}", idx))
                    .num_columns(2)
                    .spacing([16.0, 6.0])
                    .show(ui, |ui| {
                        self.render_ssh_fields(ui, idx);
                    });
                ui.add_space(SPACE_XS);
                ui.horizontal(|ui| {
                    if ui
                        .button(egui::RichText::new("Remove").color(self.semantic.danger))
                        .clicked()
                    {
                        *remove_idx = Some(idx);
                    }
                    let testing = self.ssh_test_rx.is_some();
                    let label = if testing {
                        "Testing…"
                    } else {
                        "Test Connection"
                    };
                    if ui.add_enabled(!testing, egui::Button::new(label)).clicked() {
                        self.test_ssh_connection(idx);
                    }
                });
            });
    }

    fn render_ssh_fields(&mut self, ui: &mut egui::Ui, idx: usize) {
        let server = &mut self.settings.ssh_servers[idx];

        ui.label("Name:");
        ui.add(
            egui::TextEdit::singleline(&mut server.name)
                .desired_width(200.0)
                .hint_text("My Server"),
        );
        ui.end_row();

        ui.label("Host:");
        ui.add(
            egui::TextEdit::singleline(&mut server.host)
                .desired_width(200.0)
                .hint_text("192.168.1.100 or hostname")
                .font(egui::TextStyle::Monospace),
        );
        ui.end_row();

        ui.label("Port:");
        let mut port_str = server.port.to_string();
        ui.add(
            egui::TextEdit::singleline(&mut port_str)
                .desired_width(60.0)
                .font(egui::TextStyle::Monospace),
        );
        if let Ok(p) = port_str.parse::<u16>() {
            server.port = p;
        }
        ui.end_row();

        ui.label("Username:");
        ui.add(
            egui::TextEdit::singleline(&mut server.username)
                .desired_width(200.0)
                .hint_text("user")
                .font(egui::TextStyle::Monospace),
        );
        ui.end_row();

        ui.label("Auth:");
        let current = server.auth_kind.display_name();
        egui::ComboBox::from_id_salt(format!("ssh_auth_{}", idx))
            .selected_text(current)
            .show_ui(ui, |ui| {
                for kind in SshAuthKind::all() {
                    let label = kind.display_name();
                    if ui
                        .selectable_label(server.auth_kind == *kind, label)
                        .clicked()
                    {
                        server.auth_kind = kind.clone();
                    }
                }
            });
        ui.end_row();

        match server.auth_kind {
            SshAuthKind::KeyFile => {
                ui.label("Key File:");
                ui.add(
                    egui::TextEdit::singleline(&mut server.key_path)
                        .desired_width(200.0)
                        .hint_text("~/.ssh/id_rsa")
                        .font(egui::TextStyle::Monospace),
                );
                ui.end_row();
            }
            SshAuthKind::Password => {
                ui.label("Password:");
                ui.add(
                    egui::TextEdit::singleline(&mut server.password)
                        .desired_width(200.0)
                        .password(true)
                        .font(egui::TextStyle::Monospace),
                );
                ui.end_row();
            }
            SshAuthKind::Agent => {}
        }

        ui.label("Remote Path:");
        ui.add(
            egui::TextEdit::singleline(&mut server.remote_path)
                .desired_width(200.0)
                .hint_text("~ or /home/user/project")
                .font(egui::TextStyle::Monospace),
        );
        ui.end_row();
    }

    fn test_ssh_connection(&mut self, idx: usize) {
        if self.ssh_test_rx.is_some() {
            return;
        }
        let server = &self.settings.ssh_servers[idx];
        let config = crate::ssh::SshServerConfig {
            name: server.name.clone(),
            host: server.host.clone(),
            port: server.port,
            username: server.username.clone(),
            auth_method: match &server.auth_kind {
                SshAuthKind::Agent => crate::ssh::SshAuthMethod::Agent,
                SshAuthKind::KeyFile => crate::ssh::SshAuthMethod::KeyFile {
                    path: crate::app::util::expand_tilde(&server.key_path),
                },
                SshAuthKind::Password => crate::ssh::SshAuthMethod::Password {
                    password: server.password.clone(),
                },
            },
            remote_path: server.remote_path.clone(),
        };
        let remote_path = server.remote_path.clone();
        let success_msg = format!(
            "SSH + SFTP connection to '{}' ({}@{}:{}) succeeded",
            server.name, server.username, server.host, server.remote_path
        );
        self.set_status_message("Testing SSH connection…".into());
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = self.egui_ctx.clone();
        std::thread::spawn(move || {
            let result = match crate::ssh::SshConnection::connect(&config) {
                Ok(conn) => {
                    let sftp_result = conn.list_dir(&remote_path);
                    conn.disconnect();
                    match sftp_result {
                        Ok(_) => Ok(success_msg),
                        Err(e) => Err(format!(
                            "SSH auth succeeded but SFTP failed for '{}': {}",
                            remote_path, e
                        )),
                    }
                }
                Err(e) => Err(format!("SSH connection failed: {}", e)),
            };
            let _ = tx.send(result);
            if let Some(c) = ctx.get() {
                c.request_repaint();
            }
        });
        self.ssh_test_rx = Some(rx);
    }

    pub(in crate::app) fn process_ssh_test_result(&mut self) {
        let rx = match self.ssh_test_rx.as_ref() {
            Some(rx) => rx,
            None => return,
        };
        let result = match rx.try_recv() {
            Ok(r) => r,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.ssh_test_rx = None;
                self.set_status_message("SSH test connection thread died".into());
                return;
            }
        };
        self.ssh_test_rx = None;
        match result {
            Ok(msg) => self.set_status_message(msg),
            Err(msg) => self.set_status_message(msg),
        }
    }
}
