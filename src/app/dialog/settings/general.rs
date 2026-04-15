use eframe::egui;

use crate::app::{CustomThemeEdit, DirigentApp, SPACE_MD, SPACE_SM};
use crate::settings::{CliProvider, ThemeChoice};

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
                self.render_settings_provider_row(ui);
                self.render_settings_model_row(ui, refresh_models);
                self.render_settings_cli_paths_row(ui);
                self.render_settings_font_row(ui);
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
                    let mut prev_was_dark = true;
                    for variant in ThemeChoice::all_variants() {
                        if prev_was_dark && !variant.is_dark() {
                            ui.separator();
                            prev_was_dark = false;
                        }
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
                            ui.selectable_value(
                                &mut self.settings.theme,
                                ThemeChoice::Custom(ct.clone()),
                                &ct.name,
                            );
                        }
                    }
                });
            if ui
                .small_button("+")
                .on_hover_text("New custom theme")
                .clicked()
            {
                self.custom_theme_edit = Some(CustomThemeEdit {
                    theme: self.settings.theme.to_custom_theme(),
                    editing_index: None,
                    ai_prompt: String::new(),
                    ai_generating: false,
                    ai_rx: None,
                    ai_error: None,
                });
            }
            // Edit button for current custom theme
            if let ThemeChoice::Custom(ref ct) = self.settings.theme {
                if ui
                    .small_button("\u{270E}")
                    .on_hover_text("Edit custom theme")
                    .clicked()
                {
                    let idx = self
                        .settings
                        .custom_themes
                        .iter()
                        .position(|t| t.name == ct.name);
                    self.custom_theme_edit = Some(CustomThemeEdit {
                        theme: ct.clone(),
                        editing_index: idx,
                        ai_prompt: String::new(),
                        ai_generating: false,
                        ai_rx: None,
                        ai_error: None,
                    });
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

        egui::ComboBox::from_id_salt("claude_model_combo")
            .selected_text(&self.settings.claude_model)
            .show_ui(ui, |ui| {
                for model in DEFAULT_CLAUDE_MODELS {
                    ui.selectable_value(&mut self.settings.claude_model, model.to_string(), *model);
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
                // Show the current model even if it's not in either list
                // (e.g. manually edited in settings JSON).
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
        let flags = if self.settings.allow_dangerous_skip_permissions {
            "-p <prompt> --verbose --output-format stream-json --dangerously-skip-permissions"
        } else {
            "-p <prompt> --verbose --output-format stream-json"
        };
        ui.label(egui::RichText::new(flags).monospace().weak());
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
    }

    fn render_settings_misc_rows(&mut self, ui: &mut egui::Ui) {
        ui.label("Notifications:");
        ui.end_row();

        ui.label("  Sound:");
        ui.checkbox(&mut self.settings.notify_sound, "Play sound on task review");
        ui.end_row();

        ui.label("  Popup:");
        ui.checkbox(&mut self.settings.notify_popup, "Show macOS notification");
        ui.end_row();

        ui.label("Lava Lamp:");
        ui.checkbox(
            &mut self.settings.lava_lamp_enabled,
            "Show lava lamp while running",
        );
        ui.end_row();

        ui.label("Home Folder:");
        ui.checkbox(&mut self.settings.allow_home_folder_access, "Allow AI access to home folders")
            .on_hover_text("When disabled, installs a Claude Code PreToolUse hook that blocks access to personal directories like ~/Documents, ~/Desktop, ~/Downloads, ~/Photos, ~/Music, ~/Library, ~/.ssh, etc.");
        ui.end_row();
    }
}
