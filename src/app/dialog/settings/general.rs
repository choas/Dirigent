use eframe::egui;

use crate::app::{CustomThemeEdit, DirigentApp, SPACE_MD, SPACE_SM};
use crate::settings::{
    CliProvider, DiffColorScheme, FontWeight, HeartbeatStyle, RunningAnimation, ThemeChoice,
    VcsBackend,
};

/// Render a labeled monospace text field row in a settings grid.
fn cli_field(ui: &mut egui::Ui, label: &str, value: &mut String, hint: &str) {
    ui.label(label);
    ui.add(
        egui::TextEdit::singleline(value)
            .desired_width(250.0)
            .hint_text(hint)
            .font(egui::TextStyle::Monospace),
    );
    ui.end_row();
}

impl DirigentApp {
    pub(in crate::app) fn render_settings_general_grid(
        &mut self,
        ui: &mut egui::Ui,
        refresh_models: &mut bool,
    ) {
        egui::Grid::new("settings_grid")
            .num_columns(2)
            .spacing([SPACE_MD, SPACE_SM])
            .show(ui, |ui| {
                self.render_settings_theme_row(ui);
                self.render_settings_font_row(ui);
                self.render_settings_provider_row(ui);
                self.render_settings_model_row(ui, refresh_models);
                self.render_settings_cli_paths_row(ui);
                self.render_settings_misc_rows(ui);
            });
    }

    fn render_settings_theme_row(&mut self, ui: &mut egui::Ui) {
        ui.label("Theme:");
        ui.horizontal(|ui| {
            let theme_label = self.settings.theme.display_name().to_string();
            egui::ComboBox::from_id_salt("theme_combo")
                .selected_text(&theme_label)
                .show_ui(ui, |ui| {
                    let variants = ThemeChoice::all_variants();
                    let mut prev_was_dark = variants.first().map_or(true, |v| v.is_dark());
                    for variant in variants {
                        let is_dark = variant.is_dark();
                        if prev_was_dark && !is_dark {
                            ui.separator();
                        }
                        prev_was_dark = is_dark;
                        ui.selectable_value(
                            &mut self.settings.theme,
                            variant.clone(),
                            variant.display_name(),
                        );
                    }
                    // Custom themes section
                    if !self.settings.custom_themes.is_empty() {
                        ui.separator();
                        for ct in &self.settings.custom_themes {
                            let is_selected = matches!(
                                &self.settings.theme,
                                ThemeChoice::Custom(active) if active.name == ct.name
                            );
                            if ui.selectable_label(is_selected, &ct.name).clicked() {
                                self.settings.theme = ThemeChoice::Custom(ct.clone());
                            }
                        }
                    }
                });
            if ui
                .small_button("+")
                .on_hover_text("New custom theme")
                .clicked()
            {
                self.custom_theme_edit = Some(CustomThemeEdit::new(
                    self.settings.theme.to_custom_theme(),
                    None,
                ));
            }
            // Edit button for current custom theme
            if let ThemeChoice::Custom(ref ct) = self.settings.theme {
                if ui
                    .small_button("\u{270E}")
                    .on_hover_text("Edit custom theme")
                    .clicked()
                {
                    let idx = self.settings.custom_themes.iter().position(|t| t == ct);
                    self.custom_theme_edit = Some(CustomThemeEdit::new(ct.clone(), idx));
                }
            }
        });
        ui.end_row();
    }

    fn render_settings_provider_row(&mut self, ui: &mut egui::Ui) {
        ui.label("CLI Provider:");
        let provider_label = self.settings.cli_provider.display_name();
        egui::ComboBox::from_id_salt("provider_combo")
            .selected_text(provider_label)
            .show_ui(ui, |ui| {
                for provider in CliProvider::all() {
                    ui.selectable_value(
                        &mut self.settings.cli_provider,
                        provider.clone(),
                        provider.display_name(),
                    );
                }
            });
        ui.end_row();
    }

    fn render_settings_model_row(&mut self, ui: &mut egui::Ui, refresh_models: &mut bool) {
        ui.label("Model:");
        match self.settings.cli_provider {
            CliProvider::Claude => self.render_claude_model_combo(ui),
            CliProvider::OpenCode => self.render_opencode_model_combo(ui, refresh_models),
            CliProvider::Gemini => self.render_gemini_model_combo(ui, refresh_models),
            CliProvider::Codex => self.render_codex_model_combo(ui),
        }
        ui.end_row();
    }

    fn render_claude_model_combo(&mut self, ui: &mut egui::Ui) {
        const DEFAULT_CLAUDE_MODELS: &[&str] = &[
            "claude-opus-4-6",
            "claude-opus-4-5-20251101",
            "claude-sonnet-4-6",
            "claude-sonnet-4-5-20250929",
            "claude-haiku-4-5-20251001",
        ];

        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("claude_model_combo")
                .selected_text(&self.settings.claude_model)
                .show_ui(ui, |ui| {
                    for model in DEFAULT_CLAUDE_MODELS {
                        ui.selectable_value(
                            &mut self.settings.claude_model,
                            model.to_string(),
                            *model,
                        );
                    }
                    if !self.settings.claude_custom_models.is_empty() {
                        ui.separator();
                        for model in &self.settings.claude_custom_models {
                            ui.selectable_value(
                                &mut self.settings.claude_model,
                                model.clone(),
                                model.as_str(),
                            );
                        }
                    }
                    let current = self.settings.claude_model.clone();
                    if !current.is_empty()
                        && !DEFAULT_CLAUDE_MODELS.contains(&current.as_str())
                        && !self.settings.claude_custom_models.contains(&current)
                    {
                        ui.separator();
                        ui.selectable_value(
                            &mut self.settings.claude_model,
                            current.clone(),
                            current.as_str(),
                        );
                    }
                });
            ui.add(
                egui::TextEdit::singleline(&mut self.settings.claude_model)
                    .desired_width(200.0)
                    .font(egui::TextStyle::Monospace),
            );
        });
    }

    fn render_opencode_model_combo(&mut self, ui: &mut egui::Ui, refresh_models: &mut bool) {
        let models = if self.opencode_models.is_empty() {
            vec![
                "openai/o1".to_string(),
                "openai/o1-mini".to_string(),
                "openai/o3".to_string(),
                "openai/o3-mini".to_string(),
                "openai/o4-mini".to_string(),
                "openai/gpt-4.1".to_string(),
                "anthropic/claude-sonnet-4-6".to_string(),
                "anthropic/claude-opus-4-6".to_string(),
            ]
        } else {
            self.opencode_models.clone()
        };
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("opencode_model_combo")
                .selected_text(&self.settings.opencode_model)
                .show_ui(ui, |ui| {
                    for model in &models {
                        ui.selectable_value(
                            &mut self.settings.opencode_model,
                            model.clone(),
                            model.as_str(),
                        );
                    }
                });
            if self.opencode_models_loading {
                ui.spinner();
            } else if ui
                .small_button("\u{21BB}")
                .on_hover_text("Refresh available models from OpenCode")
                .clicked()
            {
                *refresh_models = true;
            }
        });
    }

    fn render_settings_cli_paths_row(&mut self, ui: &mut egui::Ui) {
        match self.settings.cli_provider {
            CliProvider::Claude => {
                self.render_settings_claude_cli_fields(ui);
            }
            CliProvider::OpenCode => {
                self.render_settings_opencode_cli_fields(ui);
            }
            CliProvider::Gemini => {
                self.render_settings_gemini_cli_fields(ui);
            }
            CliProvider::Codex => {
                self.render_settings_codex_cli_fields(ui);
            }
        }
    }

    fn render_settings_claude_cli_fields(&mut self, ui: &mut egui::Ui) {
        cli_field(
            ui,
            "CLI Path:",
            &mut self.settings.claude_cli_path,
            "not found \u{2014} enter path to claude",
        );
        cli_field(
            ui,
            "Extra Arguments:",
            &mut self.settings.claude_extra_args,
            "e.g. --max-turns 10",
        );

        ui.label("Default Flags:");
        let flags = match (
            self.settings.claude_use_pty,
            self.settings.allow_dangerous_skip_permissions,
        ) {
            (true, true) => "(interactive TUI under PTY) --dangerously-skip-permissions",
            (true, false) => "(interactive TUI under PTY)",
            (false, true) => "-p <prompt> --dangerously-skip-permissions",
            (false, false) => "-p <prompt>",
        };
        ui.label(egui::RichText::new(flags).monospace().weak());
        ui.end_row();

        ui.label("PTY:");
        ui.checkbox(
            &mut self.settings.claude_use_pty,
            "Run Claude Code under a PTY (default)",
        )
        .on_hover_text(
            "When enabled, Claude's interactive TUI is launched under a pseudo-terminal, \
             confirmation dialogs are auto-accepted, and a Stop hook is installed to \
             detect run completion reliably. When disabled, Claude is invoked \
             in headless `-p <prompt>` mode with stdout/stderr piped directly.",
        );
        ui.end_row();

        ui.label("Yolo:");
        ui.checkbox(
            &mut self.settings.allow_dangerous_skip_permissions,
            "Append --dangerously-skip-permissions",
        );
        ui.end_row();

        cli_field(
            ui,
            "Pre-run Script:",
            &mut self.settings.claude_pre_run_script,
            "shell command before run",
        );
        cli_field(
            ui,
            "Post-run Script:",
            &mut self.settings.claude_post_run_script,
            "shell command after run",
        );
    }

    fn render_gemini_model_combo(&mut self, ui: &mut egui::Ui, refresh_models: &mut bool) {
        let models = if self.gemini_models.is_empty() {
            vec![
                "gemini-3-pro-preview".to_string(),
                "gemini-3-flash-preview".to_string(),
                "gemini-2.5-pro".to_string(),
                "gemini-2.5-flash".to_string(),
                "gemini-2.5-flash-lite".to_string(),
            ]
        } else {
            self.gemini_models.clone()
        };
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("gemini_model_combo")
                .selected_text(&self.settings.gemini_model)
                .show_ui(ui, |ui| {
                    for model in &models {
                        ui.selectable_value(
                            &mut self.settings.gemini_model,
                            model.clone(),
                            model.as_str(),
                        );
                    }
                });
            ui.add(
                egui::TextEdit::singleline(&mut self.settings.gemini_model)
                    .desired_width(200.0)
                    .font(egui::TextStyle::Monospace),
            );
            if self.gemini_models_loading {
                ui.spinner();
            } else if ui
                .small_button("\u{21BB}")
                .on_hover_text("Refresh available models from Gemini CLI")
                .clicked()
            {
                *refresh_models = true;
            }
        });
    }

    fn render_codex_model_combo(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("codex_model_combo")
                .selected_text(&self.settings.codex_model)
                .show_ui(ui, |ui| {
                    for model in ["gpt-5-codex", "gpt-5.4", "gpt-5.4-mini"] {
                        ui.selectable_value(
                            &mut self.settings.codex_model,
                            model.to_string(),
                            model,
                        );
                    }
                });
            ui.add(
                egui::TextEdit::singleline(&mut self.settings.codex_model)
                    .desired_width(200.0)
                    .font(egui::TextStyle::Monospace),
            );
        });
    }

    fn render_settings_gemini_cli_fields(&mut self, ui: &mut egui::Ui) {
        cli_field(
            ui,
            "CLI Path:",
            &mut self.settings.gemini_cli_path,
            "not found \u{2014} enter path to gemini",
        );
        cli_field(
            ui,
            "Extra Arguments:",
            &mut self.settings.gemini_extra_args,
            "e.g. --max-turns 10",
        );

        ui.label("Default Flags:");
        ui.label(
            egui::RichText::new("-y --output-format json --model <model>")
                .monospace()
                .weak(),
        );
        ui.end_row();

        cli_field(
            ui,
            "Pre-run Script:",
            &mut self.settings.gemini_pre_run_script,
            "shell command before run",
        );
        cli_field(
            ui,
            "Post-run Script:",
            &mut self.settings.gemini_post_run_script,
            "shell command after run",
        );
    }

    fn render_settings_codex_cli_fields(&mut self, ui: &mut egui::Ui) {
        cli_field(
            ui,
            "CLI Path:",
            &mut self.settings.codex_cli_path,
            "not found — enter path to codex",
        );
        cli_field(
            ui,
            "Extra Arguments:",
            &mut self.settings.codex_extra_args,
            "e.g. --sandbox workspace-write",
        );
        ui.label("Default Flags:");
        ui.label(
            egui::RichText::new("--yolo --model <model> <prompt>")
                .monospace()
                .weak(),
        );
        ui.end_row();
        cli_field(
            ui,
            "Pre-run Script:",
            &mut self.settings.codex_pre_run_script,
            "shell command before run",
        );
        cli_field(
            ui,
            "Env Variables:",
            &mut self.settings.codex_env_vars,
            "KEY=VAL, one per line",
        );
        cli_field(
            ui,
            "Post-run Script:",
            &mut self.settings.codex_post_run_script,
            "shell command after run",
        );
    }

    fn render_settings_opencode_cli_fields(&mut self, ui: &mut egui::Ui) {
        cli_field(
            ui,
            "CLI Path:",
            &mut self.settings.opencode_cli_path,
            "not found \u{2014} enter path to opencode",
        );
        cli_field(
            ui,
            "Extra Arguments:",
            &mut self.settings.opencode_extra_args,
            "e.g. --mcp-server ...",
        );

        ui.label("Default Flags:");
        ui.label(
            egui::RichText::new("--provider <provider> --model <model>")
                .monospace()
                .weak(),
        );
        ui.end_row();

        cli_field(
            ui,
            "Pre-run Script:",
            &mut self.settings.opencode_pre_run_script,
            "shell command before run",
        );
        cli_field(
            ui,
            "Post-run Script:",
            &mut self.settings.opencode_post_run_script,
            "shell command after run",
        );
    }

    fn render_settings_font_row(&mut self, ui: &mut egui::Ui) {
        ui.label("Font:");
        egui::ComboBox::from_id_salt("font_combo")
            .selected_text(&self.settings.font_family)
            .show_ui(ui, |ui| {
                for font in &[
                    "Menlo",
                    "SF Mono",
                    "Monaco",
                    "Courier New",
                    "JetBrains Mono",
                    "Fira Code",
                    "Source Code Pro",
                    "Consolas",
                ] {
                    ui.selectable_value(&mut self.settings.font_family, font.to_string(), *font);
                }
            });
        ui.end_row();

        ui.label("Font Size:");
        ui.add(egui::Slider::new(&mut self.settings.font_size, 8.0..=32.0).step_by(0.5));
        ui.end_row();

        ui.label("Font Weight:");
        egui::ComboBox::from_id_salt("font_weight_combo")
            .selected_text(self.settings.font_weight.display_name())
            .show_ui(ui, |ui| {
                for weight in FontWeight::all() {
                    ui.selectable_value(
                        &mut self.settings.font_weight,
                        weight.clone(),
                        weight.display_name(),
                    );
                }
            });
        ui.end_row();
    }

    fn render_settings_misc_rows(&mut self, ui: &mut egui::Ui) {
        ui.label("VCS Backend:");
        let prev_backend = self.settings.vcs_backend.clone();
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("vcs_backend_combo")
                .selected_text(self.settings.vcs_backend.display_name())
                .show_ui(ui, |ui| {
                    for backend in VcsBackend::all() {
                        ui.selectable_value(
                            &mut self.settings.vcs_backend,
                            backend.clone(),
                            backend.display_name(),
                        );
                    }
                });
            if self.settings.vcs_backend == VcsBackend::Jj
                && !self.settings.jj_cli_path.is_empty()
                && !std::path::Path::new(&self.settings.jj_cli_path).exists()
            {
                ui.label(
                    egui::RichText::new("binary not found at configured path")
                        .small()
                        .color(self.semantic.danger),
                );
            }
        });
        if self.settings.vcs_backend == VcsBackend::Jj && prev_backend != VcsBackend::Jj {
            self.ensure_jj_colocated();
        }
        ui.end_row();

        if self.settings.vcs_backend == VcsBackend::Jj {
            ui.label("jj CLI Path:");
            ui.add(
                egui::TextEdit::singleline(&mut self.settings.jj_cli_path)
                    .desired_width(250.0)
                    .hint_text("leave empty to auto-detect")
                    .font(egui::TextStyle::Monospace),
            );
            ui.end_row();
        }

        ui.label("Notifications:");
        ui.end_row();

        ui.label("  Sound:");
        ui.checkbox(&mut self.settings.notify_sound, "Play sound on task review");
        ui.end_row();

        ui.label("  Popup:");
        ui.checkbox(&mut self.settings.notify_popup, "Show macOS notification");
        ui.end_row();

        ui.label("Animation:");
        egui::ComboBox::from_id_salt("running_animation_combo")
            .selected_text(self.settings.running_animation.display_name())
            .show_ui(ui, |ui| {
                for anim in RunningAnimation::all() {
                    ui.selectable_value(
                        &mut self.settings.running_animation,
                        anim.clone(),
                        anim.display_name(),
                    );
                }
            });
        ui.end_row();

        if self.settings.running_animation == RunningAnimation::ClaudeCodeName {
            ui.label("  Name:");
            ui.add(
                egui::TextEdit::singleline(&mut self.settings.claude_code_display_name)
                    .desired_width(160.0)
                    .hint_text("optional"),
            );
            ui.end_row();
        }

        ui.label("Heart Beat:");
        egui::ComboBox::from_id_salt("heartbeat_style_combo")
            .selected_text(self.settings.heartbeat_style.display_name())
            .show_ui(ui, |ui| {
                for style in HeartbeatStyle::all() {
                    ui.selectable_value(
                        &mut self.settings.heartbeat_style,
                        style.clone(),
                        style.display_name(),
                    );
                }
            });
        ui.end_row();

        ui.label("Diff Colors:");
        egui::ComboBox::from_id_salt("diff_color_combo")
            .selected_text(self.settings.diff_color_scheme.display_name())
            .show_ui(ui, |ui| {
                for scheme in DiffColorScheme::all() {
                    ui.selectable_value(
                        &mut self.settings.diff_color_scheme,
                        scheme.clone(),
                        scheme.display_name(),
                    );
                }
            });
        ui.end_row();

        ui.label("Home Folder:");
        ui.checkbox(&mut self.settings.allow_home_folder_access, "Allow AI access to home folders")
            .on_hover_text("When disabled, installs a Claude Code PreToolUse hook that blocks access to personal directories like ~/Documents, ~/Desktop, ~/Downloads, ~/Photos, ~/Music, ~/Library, ~/.ssh, etc.");
        ui.end_row();

        ui.label("Frame Timing:");
        ui.checkbox(
            &mut self.settings.show_frame_timing,
            "Show frame timing in status bar",
        )
        .on_hover_text("Display per-frame timing breakdown and memory usage in the status bar.");
        ui.end_row();

        ui.label("Dock Icon:");
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.settings.custom_dock_icon_path)
                    .desired_width(200.0)
                    .hint_text("default logo")
                    .font(egui::TextStyle::Monospace),
            );
            if ui.small_button("Browse\u{2026}").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Images", &["png", "jpg", "jpeg", "ico", "icns"])
                    .pick_file()
                {
                    self.settings.custom_dock_icon_path = path.to_string_lossy().to_string();
                }
            }
            if !self.settings.custom_dock_icon_path.is_empty()
                && ui
                    .small_button("\u{2715}")
                    .on_hover_text("Reset to default logo")
                    .clicked()
            {
                self.settings.custom_dock_icon_path.clear();
            }
        });
        ui.end_row();
    }
}
