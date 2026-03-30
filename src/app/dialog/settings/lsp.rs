use eframe::egui;

use crate::app::{icon, DirigentApp, SPACE_MD, SPACE_SM, SPACE_XS};
use crate::lsp::{
    default_lsp_servers, lsp_install_hint, lsp_servers_for_language, LspLanguage, LspServerConfig,
};

#[derive(Default)]
struct LspCardActions {
    delete_idx: Option<usize>,
    start_idx: Option<usize>,
    stop_id: Option<String>,
    install_server_name: Option<String>,
}

impl DirigentApp {
    pub(in crate::app) fn render_settings_lsp_section(&mut self, ui: &mut egui::Ui, fs: f32) {
        ui.add_space(SPACE_MD);
        ui.separator();
        ui.add_space(SPACE_SM);
        self.render_lsp_header(ui, fs);
        if !self.lsp_expanded {
            return;
        }
        ui.add_space(SPACE_SM);
        self.render_lsp_master_toggle(ui);
        ui.add_space(SPACE_SM);
        self.render_lsp_action_buttons(ui);
        ui.add_space(SPACE_SM);
        let actions = self.render_lsp_server_cards(ui, fs);
        self.apply_lsp_card_actions(actions);
        self.render_lsp_language_init(ui);
        self.render_lsp_status_log(ui);
    }

    fn log_lsp_error(&mut self, result: Result<(), String>) {
        if let Err(e) = result {
            eprintln!("[lsp] {}", e);
            self.lsp.status_log.push(e);
        }
    }

    fn render_lsp_header(&mut self, ui: &mut egui::Ui, fs: f32) {
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
    }

    fn render_lsp_master_toggle(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("lsp_master_grid")
            .num_columns(2)
            .spacing([SPACE_MD, SPACE_SM])
            .show(ui, |ui| {
                ui.label("LSP Enabled:");
                if ui
                    .checkbox(
                        &mut self.settings.lsp_enabled,
                        "Start language servers on launch",
                    )
                    .on_hover_text(
                        "When enabled, Dirigent spawns configured language servers for code intelligence (hover, go-to-definition, diagnostics).",
                    )
                    .changed()
                {
                    if self.settings.lsp_enabled {
                        let result = self.lsp.start_servers(&self.settings.lsp_servers);
                        self.log_lsp_error(result);
                    } else {
                        self.lsp.stop_all();
                    }
                }
                ui.end_row();
            });
    }

    fn render_lsp_action_buttons(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.small_button("+ Add Server").clicked() {
                self.settings.lsp_servers.push(LspServerConfig {
                    id: uuid::Uuid::new_v4().to_string(),
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
                if self.settings.lsp_enabled {
                    let result = self.lsp.reconcile(&self.settings.lsp_servers);
                    self.log_lsp_error(result);
                }
            }
            if self.settings.lsp_enabled {
                if ui
                    .small_button("\u{21BB} Restart All")
                    .on_hover_text("Stop and restart all enabled servers")
                    .clicked()
                {
                    let result = self.lsp.restart_all(&self.settings.lsp_servers);
                    self.log_lsp_error(result);
                }
            }
        });
    }

    fn render_lsp_server_cards(&mut self, ui: &mut egui::Ui, fs: f32) -> LspCardActions {
        let card_width = ui.available_width();
        let mut actions = LspCardActions::default();
        let num_servers = self.settings.lsp_servers.len();
        for i in 0..num_servers {
            self.render_lsp_server_card(ui, i, card_width, fs, &mut actions);
            ui.add_space(SPACE_XS);
        }
        actions
    }

    fn render_lsp_server_card(
        &mut self,
        ui: &mut egui::Ui,
        i: usize,
        card_width: f32,
        fs: f32,
        actions: &mut LspCardActions,
    ) {
        let running_servers = self.lsp.running_servers();
        let starting_servers = self.lsp.starting_servers();
        let server_id = &self.settings.lsp_servers[i].id;
        let is_running = running_servers.contains(server_id);
        let is_starting = starting_servers.contains(server_id);
        let server_error = self.lsp.failed_servers.get(server_id).cloned();
        let has_error = server_error.is_some();

        let mut frame = self.semantic.card_frame();
        if is_running {
            frame = frame.fill(self.semantic.addition_bg());
        } else if has_error {
            frame = frame.fill(self.semantic.deletion_bg());
        }

        frame.show(ui, |ui| {
            ui.set_width(card_width - SPACE_MD - 20.0);
            self.render_lsp_server_header_row(
                ui,
                i,
                is_running,
                is_starting,
                has_error,
                fs,
                actions,
            );
            if let Some(ref err) = server_error {
                ui.add_space(SPACE_XS);
                ui.label(egui::RichText::new(err).small().color(self.semantic.danger));
            }
            ui.add_space(SPACE_XS);
            self.render_lsp_server_detail_fields(ui, i);
        });
    }

    fn render_lsp_server_header_row(
        &mut self,
        ui: &mut egui::Ui,
        i: usize,
        is_running: bool,
        is_starting: bool,
        has_error: bool,
        fs: f32,
        actions: &mut LspCardActions,
    ) {
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.settings.lsp_servers[i].enabled, "");
            ui.add(
                egui::TextEdit::singleline(&mut self.settings.lsp_servers[i].name)
                    .desired_width(140.0)
                    .font(egui::TextStyle::Monospace),
            );
            self.render_lsp_server_status(ui, is_running, is_starting, has_error);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.render_lsp_server_action_buttons(ui, i, is_running, has_error, fs, actions);
            });
        });
    }

    fn render_lsp_server_status(
        &self,
        ui: &mut egui::Ui,
        is_running: bool,
        is_starting: bool,
        has_error: bool,
    ) {
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
    }

    fn render_lsp_server_action_buttons(
        &mut self,
        ui: &mut egui::Ui,
        i: usize,
        is_running: bool,
        has_error: bool,
        fs: f32,
        actions: &mut LspCardActions,
    ) {
        if ui
            .small_button(icon("\u{1F5D1}", fs))
            .on_hover_text("Remove this server")
            .clicked()
        {
            actions.delete_idx = Some(i);
        }
        if is_running {
            if ui.small_button("Stop").clicked() {
                actions.stop_id = Some(self.settings.lsp_servers[i].id.clone());
            }
        } else if has_error {
            self.render_lsp_failed_server_actions(ui, i, actions);
        } else if self.settings.lsp_enabled && self.settings.lsp_servers[i].enabled {
            if ui.small_button("Start").clicked() {
                actions.start_idx = Some(i);
            }
        }
    }

    fn render_lsp_failed_server_actions(
        &self,
        ui: &mut egui::Ui,
        i: usize,
        actions: &mut LspCardActions,
    ) {
        if ui
            .small_button("\u{1F4E6} Install")
            .on_hover_text("Create a cue with instructions to install this LSP server")
            .clicked()
        {
            actions.install_server_name = Some(self.settings.lsp_servers[i].name.clone());
        }
        if self.settings.lsp_enabled && self.settings.lsp_servers[i].enabled {
            if ui
                .small_button("\u{21BB} Retry")
                .on_hover_text("Try starting the server again")
                .clicked()
            {
                actions.start_idx = Some(i);
            }
        }
    }

    fn render_lsp_server_detail_fields(&mut self, ui: &mut egui::Ui, i: usize) {
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
                let mut args_str =
                    shlex::try_join(self.settings.lsp_servers[i].args.iter().map(|s| s.as_str()))
                        .unwrap_or_default();
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut args_str)
                        .desired_width(250.0)
                        .hint_text("e.g. --stdio (quoting supported)")
                        .font(egui::TextStyle::Monospace),
                );
                if resp.changed() {
                    match shlex::split(&args_str) {
                        Some(parsed) => {
                            self.settings.lsp_servers[i].args = parsed;
                            self.lsp_args_parse_warnings
                                .remove(&self.settings.lsp_servers[i].id);
                        }
                        None => {
                            self.settings.lsp_servers[i].args =
                                args_str.split_whitespace().map(|s| s.to_string()).collect();
                            self.lsp_args_parse_warnings.insert(
                                self.settings.lsp_servers[i].id.clone(),
                                "Malformed quoting in arguments".to_string(),
                            );
                        }
                    }
                }
                if let Some(warning) = self
                    .lsp_args_parse_warnings
                    .get(&self.settings.lsp_servers[i].id)
                {
                    ui.end_row();
                    ui.label("");
                    ui.label(
                        egui::RichText::new(format!("\u{26A0} {}", warning))
                            .color(egui::Color32::from_rgb(255, 180, 0))
                            .small(),
                    );
                }
                ui.end_row();

                ui.label("Env Vars:");
                // Normalize: flatten any entries that contain embedded newlines
                // (legacy corruption from join/split mismatch).
                let env = &mut self.settings.lsp_servers[i].env;
                if env.iter().any(|s| s.contains('\n')) {
                    *env = env
                        .iter()
                        .flat_map(|s| s.split('\n'))
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
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
    }

    fn apply_lsp_card_actions(&mut self, actions: LspCardActions) {
        if let Some(idx) = actions.delete_idx {
            let id = self.settings.lsp_servers[idx].id.clone();
            self.lsp.stop_server(&id);
            self.lsp.failed_servers.remove(&id);
            self.lsp_args_parse_warnings.remove(&id);
            self.settings.lsp_servers.remove(idx);
        }
        if let Some(id) = actions.stop_id {
            self.lsp.stop_server(&id);
        }
        if let Some(idx) = actions.start_idx {
            let cfg = self.settings.lsp_servers[idx].clone();
            let result = self.lsp.start_single(&cfg);
            self.log_lsp_error(result);
        }
        if let Some(name) = actions.install_server_name {
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
    }

    fn render_lsp_language_init(&mut self, ui: &mut egui::Ui) {
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
                if self.settings.lsp_enabled {
                    let result = self.lsp.reconcile(&self.settings.lsp_servers);
                    self.log_lsp_error(result);
                }
            }
        });
    }

    fn render_lsp_status_log(&self, ui: &mut egui::Ui) {
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
