use eframe::egui;

use crate::app::{icon, DirigentApp, SPACE_MD, SPACE_SM, SPACE_XS};
use crate::lsp::{
    default_lsp_servers, lsp_install_hint, lsp_servers_for_language, LspLanguage, LspServerConfig,
};

impl DirigentApp {
    pub(in crate::app) fn render_settings_lsp_section(&mut self, ui: &mut egui::Ui, fs: f32) {
        ui.add_space(SPACE_MD);
        ui.separator();
        ui.add_space(SPACE_SM);
        ui.horizontal(|ui| {
            let arrow = ["\u{25B6}", "\u{25BC}"][self.lsp_expanded as usize];
            if ui
                .button(icon(&format!("{} Language Servers (LSP)", arrow), fs))
                .clicked()
            {
                self.lsp_expanded = !self.lsp_expanded;
            }
            let running = self.lsp.running_servers().len();
            let total = self
                .settings
                .lsp_servers
                .iter()
                .filter(|s| s.enabled)
                .count();
            ui.label(
                egui::RichText::new(format!("{}/{} running", running, total))
                    .small()
                    .color(self.semantic.secondary_text),
            );
        });

        if !self.lsp_expanded {
            return;
        }

        ui.add_space(SPACE_SM);

        // Master toggle
        egui::Grid::new("lsp_master_grid")
            .num_columns(2)
            .spacing([SPACE_MD, SPACE_SM])
            .show(ui, |ui| {
                ui.label("LSP Enabled:");
                if ui
                    .checkbox(&mut self.settings.lsp_enabled, "Start language servers on launch")
                    .on_hover_text(
                        "When enabled, Dirigent spawns configured language servers for code intelligence (hover, go-to-definition, diagnostics).",
                    )
                    .changed()
                {
                    if self.settings.lsp_enabled {
                        self.lsp.start_servers(&self.settings.lsp_servers);
                    } else {
                        self.lsp.stop_all();
                    }
                }
                ui.end_row();
            });

        ui.add_space(SPACE_SM);

        // Action buttons
        ui.horizontal(|ui| {
            if ui.small_button("+ Add Server").clicked() {
                self.settings.lsp_servers.push(LspServerConfig {
                    name: "new-server".into(),
                    extensions: vec![],
                    command: String::new(),
                    args: vec![],
                    env: vec![],
                    enabled: false,
                });
            }
            if ui.small_button("Reset Defaults").clicked() {
                self.settings.lsp_servers = default_lsp_servers();
            }
            if self.settings.lsp_enabled {
                if ui
                    .small_button("\u{21BB} Restart All")
                    .on_hover_text("Stop and restart all enabled servers")
                    .clicked()
                {
                    self.lsp.stop_all();
                    self.lsp.start_servers(&self.settings.lsp_servers);
                }
            }
        });

        ui.add_space(SPACE_SM);

        // Server cards
        let card_width = ui.available_width();
        let mut delete_idx: Option<usize> = None;
        let mut start_idx: Option<usize> = None;
        let mut stop_name: Option<String> = None;
        let mut install_server_name: Option<String> = None;
        let num_servers = self.settings.lsp_servers.len();

        for i in 0..num_servers {
            let running_servers = self.lsp.running_servers();
            let starting_servers = self.lsp.starting_servers();
            let server_name = &self.settings.lsp_servers[i].name;
            let is_running = running_servers.contains(server_name);
            let is_starting = starting_servers.contains(server_name);
            let server_error = self.lsp.failed_servers.get(server_name).cloned();
            let has_error = server_error.is_some();

            let mut frame = self.semantic.card_frame();
            if is_running {
                frame = frame.fill(self.semantic.addition_bg());
            } else if has_error {
                frame = frame.fill(self.semantic.deletion_bg());
            }

            frame.show(ui, |ui| {
                ui.set_width(card_width - SPACE_MD - 20.0);

                // Header row: enabled toggle + name + status + actions
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.settings.lsp_servers[i].enabled, "");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings.lsp_servers[i].name)
                            .desired_width(140.0)
                            .font(egui::TextStyle::Monospace),
                    );

                    // Status indicator
                    if is_running {
                        ui.label(
                            egui::RichText::new("\u{25CF} running")
                                .small()
                                .color(self.semantic.success),
                        );
                    } else if is_starting {
                        ui.spinner();
                        ui.label(
                            egui::RichText::new("starting...")
                                .small()
                                .color(self.semantic.secondary_text),
                        );
                    } else if has_error {
                        ui.label(
                            egui::RichText::new("\u{25CF} failed")
                                .small()
                                .color(self.semantic.danger),
                        );
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .small_button(icon("\u{1F5D1}", fs))
                            .on_hover_text("Remove this server")
                            .clicked()
                        {
                            delete_idx = Some(i);
                        }
                        if is_running {
                            if ui.small_button("Stop").clicked() {
                                stop_name = Some(self.settings.lsp_servers[i].name.clone());
                            }
                        } else if has_error {
                            if ui
                                .small_button("\u{1F4E6} Install")
                                .on_hover_text(
                                    "Create a cue with instructions to install this LSP server",
                                )
                                .clicked()
                            {
                                install_server_name =
                                    Some(self.settings.lsp_servers[i].name.clone());
                            }
                            if self.settings.lsp_enabled && self.settings.lsp_servers[i].enabled {
                                if ui
                                    .small_button("\u{21BB} Retry")
                                    .on_hover_text("Try starting the server again")
                                    .clicked()
                                {
                                    start_idx = Some(i);
                                }
                            }
                        } else if self.settings.lsp_enabled && self.settings.lsp_servers[i].enabled
                        {
                            if ui.small_button("Start").clicked() {
                                start_idx = Some(i);
                            }
                        }
                    });
                });

                // Show error message if present
                if let Some(ref err) = server_error {
                    ui.add_space(SPACE_XS);
                    ui.label(egui::RichText::new(err).small().color(self.semantic.danger));
                }

                ui.add_space(SPACE_XS);

                // Detail fields
                egui::Grid::new(format!("lsp_server_{}", i))
                    .num_columns(2)
                    .spacing([SPACE_SM, SPACE_XS])
                    .show(ui, |ui| {
                        ui.label("Command:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.settings.lsp_servers[i].command)
                                .desired_width(250.0)
                                .hint_text("e.g. rust-analyzer")
                                .font(egui::TextStyle::Monospace),
                        );
                        ui.end_row();

                        ui.label("Extensions:");
                        let mut ext_str = self.settings.lsp_servers[i].extensions.join(", ");
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut ext_str)
                                .desired_width(250.0)
                                .hint_text("e.g. rs, toml")
                                .font(egui::TextStyle::Monospace),
                        );
                        if resp.changed() {
                            self.settings.lsp_servers[i].extensions = ext_str
                                .split(',')
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty())
                                .collect();
                        }
                        ui.end_row();

                        ui.label("Arguments:");
                        let mut args_str = shlex::join(
                            self.settings.lsp_servers[i].args.iter().map(|s| s.as_str()),
                        );
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut args_str)
                                .desired_width(250.0)
                                .hint_text("e.g. --stdio (quoting supported)")
                                .font(egui::TextStyle::Monospace),
                        );
                        if resp.changed() {
                            self.settings.lsp_servers[i].args = shlex::split(&args_str)
                                .unwrap_or_else(|| {
                                    args_str.split_whitespace().map(|s| s.to_string()).collect()
                                });
                        }
                        ui.end_row();

                        ui.label("Env Vars:");
                        let mut env_str = self.settings.lsp_servers[i].env.join(", ");
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut env_str)
                                .desired_width(250.0)
                                .hint_text("KEY=VALUE, KEY2=VALUE2")
                                .font(egui::TextStyle::Monospace),
                        );
                        if resp.changed() {
                            self.settings.lsp_servers[i].env = env_str
                                .split([',', '\n'])
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty())
                                .collect();
                        }
                        ui.end_row();
                    });
            });

            ui.add_space(SPACE_XS);
        }

        // Apply deferred actions
        if let Some(idx) = delete_idx {
            let name = self.settings.lsp_servers[idx].name.clone();
            self.lsp.stop_server(&name);
            self.lsp.failed_servers.remove(&name);
            self.settings.lsp_servers.remove(idx);
        }
        if let Some(name) = stop_name {
            self.lsp.stop_server(&name);
        }
        if let Some(idx) = start_idx {
            let cfg = self.settings.lsp_servers[idx].clone();
            self.lsp.start_single(&cfg);
        }
        if let Some(name) = install_server_name {
            let hint = lsp_install_hint(&name);
            let prompt = format!(
                "Install the `{}` language server so it is available on PATH.\n\n{}",
                name, hint
            );
            if let Ok(cue_id) = self.db.insert_cue(&prompt, "", 0, None, &[]) {
                self.reload_cues();
                self.set_status_message(format!("Created install cue for {} (#{cue_id})", name));
            }
        }

        // Language initialization dropdown
        ui.add_space(SPACE_SM);
        ui.horizontal(|ui| {
            ui.label("Language:");
            egui::ComboBox::from_id_salt("lsp_init_language")
                .selected_text(self.lsp_init_language.label())
                .show_ui(ui, |ui| {
                    for lang in LspLanguage::all() {
                        ui.selectable_value(&mut self.lsp_init_language, *lang, lang.label());
                    }
                });
            if ui.button("Initialize").clicked() {
                self.settings.lsp_servers = lsp_servers_for_language(self.lsp_init_language);
            }
        });

        // Show LSP status log (last few entries)
        if !self.lsp.status_log.is_empty() {
            ui.add_space(SPACE_SM);
            ui.label(
                egui::RichText::new("Status Log")
                    .small()
                    .color(self.semantic.secondary_text),
            );
            let start = self.lsp.status_log.len().saturating_sub(5);
            for entry in &self.lsp.status_log[start..] {
                ui.label(egui::RichText::new(entry).small().monospace());
            }
        }
    }
}
