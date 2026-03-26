use eframe::egui;

use super::super::{DirigentApp, SPACE_MD, SPACE_SM, SPACE_XS};
use crate::agents::{AgentKind, AgentStatus};
use crate::settings::SemanticColors;

/// Actions deferred from the menu bar closures.
#[derive(Default)]
struct MenuBarActions {
    push_clicked: bool,
    pull_clicked: bool,
    create_pr_clicked: bool,
    import_pr_clicked: bool,
    run_all_agents: bool,
    agent_to_trigger: Option<AgentKind>,
    agent_to_cancel: Option<AgentKind>,
}

impl DirigentApp {
    pub(in super::super) fn render_menu_bar(&mut self, ctx: &egui::Context) {
        let mut actions = MenuBarActions::default();

        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                self.render_dirigent_menu(ui);
                self.render_git_menu(ui, &mut actions);
                self.render_agents_menu(ui, &mut actions);
            });
        });

        self.apply_menu_bar_actions(actions);
    }

    fn render_dirigent_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Dirigent", |ui| {
            if ui.button("About Dirigent").clicked() {
                self.show_about = true;
                ui.close();
            }
            ui.separator();
            if ui.button("New Window  \u{2318}N").clicked() {
                crate::spawn_new_instance();
                ui.close();
            }
            ui.separator();
            if ui.button("Settings...").clicked() {
                self.dismiss_central_overlays();
                self.reload_settings_from_disk();
                self.show_settings = true;
                ui.close();
            }
        });
    }

    fn render_git_menu(&mut self, ui: &mut egui::Ui, actions: &mut MenuBarActions) {
        ui.menu_button("Git", |ui| {
            if self.git.info.is_none() {
                ui.label(
                    egui::RichText::new("No git repository")
                        .italics()
                        .color(self.semantic.tertiary_text),
                );
                return;
            }

            if let Some(ref info) = self.git.info {
                ui.label(egui::RichText::new(format!("\u{25CF} {}", info.branch)).strong());
                ui.separator();
            }

            self.render_git_menu_pull_push(ui, actions);

            ui.separator();

            self.render_git_menu_pr(ui, actions);
        });
    }

    fn render_git_menu_pull_push(&self, ui: &mut egui::Ui, actions: &mut MenuBarActions) {
        let pull_label = if self.git.pulling {
            "Pulling..."
        } else {
            "Pull"
        };
        if ui
            .add_enabled(!self.git.pulling, egui::Button::new(pull_label))
            .clicked()
        {
            actions.pull_clicked = true;
            ui.close();
        }

        if self.git.ahead_of_remote == 0 && !self.git.pushing {
            ui.add_enabled(false, egui::Button::new("  Nothing to push  "));
        } else {
            let push_label = if self.git.pushing {
                "Pushing..."
            } else {
                "Push"
            };
            if ui
                .add_enabled(!self.git.pushing, egui::Button::new(push_label))
                .clicked()
            {
                actions.push_clicked = true;
                ui.close();
            }
        }
    }

    fn render_git_menu_pr(&self, ui: &mut egui::Ui, actions: &mut MenuBarActions) {
        let is_default_branch = self
            .git
            .info
            .as_ref()
            .map(|i| i.branch == "main" || i.branch == "master")
            .unwrap_or(true);
        let pr_label = if self.git.creating_pr {
            "Creating PR..."
        } else {
            "Create Pull Request"
        };
        let pr_enabled = !self.git.creating_pr && !is_default_branch;
        if ui
            .add_enabled(pr_enabled, egui::Button::new(pr_label))
            .clicked()
        {
            actions.create_pr_clicked = true;
            ui.close();
        }

        let import_label = if self.git.importing_pr {
            "Importing PR..."
        } else {
            "Import PR Findings"
        };
        if ui
            .add_enabled(!self.git.importing_pr, egui::Button::new(import_label))
            .clicked()
        {
            actions.import_pr_clicked = true;
            ui.close();
        }
    }

    fn render_agents_menu(&mut self, ui: &mut egui::Ui, actions: &mut MenuBarActions) {
        ui.menu_button("Agents", |ui| {
            let enabled_agents: Vec<_> = self
                .settings
                .agents
                .iter()
                .filter(|a| a.enabled && !a.command.is_empty())
                .map(|a| {
                    let status = self
                        .agent_state
                        .statuses
                        .get(&a.kind)
                        .copied()
                        .unwrap_or(AgentStatus::Idle);
                    (
                        a.kind,
                        a.display_name().to_string(),
                        a.command.clone(),
                        status,
                    )
                })
                .collect();

            if enabled_agents.is_empty() {
                self.render_agents_menu_empty(ui);
                return;
            }

            self.render_agents_menu_run_all(ui, &enabled_agents, actions);
            ui.separator();
            Self::render_agents_menu_items(ui, &enabled_agents, &self.semantic, actions);

            ui.separator();
            if ui.button("Settings...").clicked() {
                self.dismiss_central_overlays();
                self.reload_settings_from_disk();
                self.show_settings = true;
                self.agents_expanded = true;
                ui.close();
            }
        });
    }

    fn render_agents_menu_empty(&mut self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new("No agents configured")
                .italics()
                .color(self.semantic.tertiary_text),
        );
        ui.separator();
        if ui.button("Open Settings...").clicked() {
            self.dismiss_central_overlays();
            self.reload_settings_from_disk();
            self.show_settings = true;
            self.agents_expanded = true;
            ui.close();
        }
    }

    fn render_agents_menu_run_all(
        &self,
        ui: &mut egui::Ui,
        enabled_agents: &[(AgentKind, String, String, AgentStatus)],
        actions: &mut MenuBarActions,
    ) {
        let any_idle = enabled_agents
            .iter()
            .any(|(_, _, _, s)| *s != AgentStatus::Running);
        if ui
            .add_enabled(any_idle, egui::Button::new("Run All"))
            .clicked()
        {
            actions.run_all_agents = true;
            ui.close();
        }
    }

    fn render_agents_menu_items(
        ui: &mut egui::Ui,
        enabled_agents: &[(AgentKind, String, String, AgentStatus)],
        semantic: &SemanticColors,
        actions: &mut MenuBarActions,
    ) {
        for (kind, name, command, status) in enabled_agents {
            let (status_icon, status_color) = match status {
                AgentStatus::Idle => ("", semantic.secondary_text),
                AgentStatus::Running => ("\u{21BB} ", semantic.accent),
                AgentStatus::Passed => ("\u{2713} ", semantic.success),
                AgentStatus::Failed => ("\u{2717} ", semantic.danger),
                AgentStatus::Error => ("! ", semantic.danger),
            };

            let is_running = *status == AgentStatus::Running;
            let label = format!("{}{}", status_icon, name);

            if is_running {
                if ui
                    .button(egui::RichText::new(&label).color(status_color))
                    .on_hover_text(format!("Cancel {}", name))
                    .clicked()
                {
                    actions.agent_to_cancel = Some(*kind);
                    ui.close();
                }
            } else if ui.button(&label).on_hover_text(command).clicked() {
                actions.agent_to_trigger = Some(*kind);
                ui.close();
            }
        }
    }

    fn apply_menu_bar_actions(&mut self, actions: MenuBarActions) {
        if actions.pull_clicked {
            self.start_git_pull();
        }
        if actions.push_clicked {
            self.start_git_push();
        }
        if actions.create_pr_clicked {
            self.open_create_pr_dialog();
        }
        if actions.import_pr_clicked {
            self.open_import_pr_dialog();
        }
        if let Some(kind) = actions.agent_to_cancel {
            self.cancel_agent(kind);
        }
        if actions.run_all_agents {
            self.run_all_agents();
        } else if let Some(kind) = actions.agent_to_trigger {
            self.trigger_agent_manual(kind);
        }
    }

    /// Trigger all enabled agents that are not currently running.
    fn run_all_agents(&mut self) {
        let kinds: Vec<AgentKind> = self
            .settings
            .agents
            .iter()
            .filter(|a| {
                a.enabled
                    && !a.command.is_empty()
                    && self.agent_state.statuses.get(&a.kind).copied() != Some(AgentStatus::Running)
            })
            .map(|a| a.kind)
            .collect();
        for kind in kinds {
            self.trigger_agent_manual(kind);
        }
    }

    pub(in super::super) fn render_about_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_about {
            return;
        }

        self.ensure_logo_texture(ctx);

        let mut open = self.show_about;
        egui::Window::new("About Dirigent")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .title_bar(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.about_dialog_frame())
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    if let Some(ref tex) = self.logo_texture {
                        ui.add(egui::Image::new(tex).max_size(egui::vec2(128.0, 128.0)));
                    }
                    ui.add_space(SPACE_MD);
                    ui.heading("Dirigent");
                    ui.add_space(SPACE_XS);
                    ui.label(format!("Version {}", env!("BUILD_VERSION")));
                    ui.add_space(SPACE_SM);
                    ui.label(
                        egui::RichText::new(
                            "A read-only code viewer where humans direct and AI performs.",
                        )
                        .weak(),
                    );
                    ui.add_space(24.0);
                });
            });
        self.show_about = open;
    }
}
