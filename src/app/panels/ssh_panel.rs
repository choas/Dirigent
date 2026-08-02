use eframe::egui;

use super::super::{DirigentApp, FONT_SCALE_SUBHEADING, SPACE_SM};
use super::file_tree::{allocate_tree_row, paint_hover_highlight};

#[derive(Default)]
struct SshEntryActions {
    navigate_to: Option<String>,
    open_file: Option<String>,
}

impl DirigentApp {
    pub(in super::super) fn render_ssh_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("ssh_panel")
            .default_size(260.0)
            .min_size(180.0)
            .max_size(500.0)
            .show_inside(ui, |ui| {
                self.render_ssh_panel_header(ui);
                ui.separator();
                self.render_ssh_path_bar(ui);
                ui.separator();
                self.render_ssh_entries(ui);
            });
    }

    fn render_ssh_panel_header(&mut self, ui: &mut egui::Ui) {
        let fs = self.settings.font_size * FONT_SCALE_SUBHEADING;
        ui.horizontal(|ui| {
            if ui
                .selectable_label(false, egui::RichText::new("Files").size(fs))
                .clicked()
            {
                self.show_ssh_panel = false;
            }
            let conn_name = self
                .ssh_worker
                .as_ref()
                .map(|w| w.config.name.clone())
                .unwrap_or_else(|| "Remote".into());
            let _ = ui.selectable_label(
                true,
                egui::RichText::new(format!("SSH: {}", conn_name)).size(fs),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Disconnect").clicked() {
                    self.ssh_disconnect();
                }
            });
        });
    }

    fn render_ssh_path_bar(&mut self, ui: &mut egui::Ui) {
        let mut navigate_to: Option<String> = None;

        ui.horizontal(|ui| {
            if ui
                .small_button("\u{2191}")
                .on_hover_text("Parent directory")
                .clicked()
            {
                if let Some(parent) = parent_path(&self.ssh_remote_path) {
                    navigate_to = Some(parent);
                }
            }
            ui.label(
                egui::RichText::new(&self.ssh_remote_path)
                    .monospace()
                    .small(),
            );
        });

        if let Some(path) = navigate_to {
            self.ssh_list_dir(&path);
        }
    }

    fn render_ssh_entries(&mut self, ui: &mut egui::Ui) {
        if self.render_ssh_blocking_state(ui) {
            return;
        }

        self.render_ssh_reading_indicator(ui);

        if self.ssh_remote_entries.is_empty() {
            self.render_ssh_empty_directory(ui);
            return;
        }

        let actions = self.render_ssh_entry_list(ui);
        self.apply_ssh_entry_actions(actions);
    }

    fn render_ssh_blocking_state(&self, ui: &mut egui::Ui) -> bool {
        if self.ssh_connect_op.in_flight() {
            render_ssh_spinner(ui, "Connecting...");
            return true;
        }
        if self.ssh_worker.is_none() {
            ui.add_space(SPACE_SM);
            ui.label(
                egui::RichText::new("Not connected")
                    .italics()
                    .color(self.semantic.tertiary_text),
            );
            return true;
        }
        if self.ssh_listing {
            render_ssh_spinner(ui, "Loading...");
            return true;
        }
        false
    }

    fn render_ssh_reading_indicator(&self, ui: &mut egui::Ui) {
        if self.ssh_reading_file.is_some() {
            render_ssh_spinner(ui, "Reading file...");
        }
    }

    fn render_ssh_empty_directory(&self, ui: &mut egui::Ui) {
        ui.add_space(SPACE_SM);
        ui.label(
            egui::RichText::new("(empty directory)")
                .italics()
                .color(self.semantic.tertiary_text),
        );
    }

    fn render_ssh_entry_list(&self, ui: &mut egui::Ui) -> SshEntryActions {
        let mut actions = SshEntryActions::default();
        egui::ScrollArea::vertical()
            .id_salt("ssh_remote_tree")
            .show(ui, |ui| {
                let entries = self.ssh_remote_entries.clone();
                for entry in &entries {
                    self.render_ssh_entry_row(ui, entry, &mut actions);
                }
            });
        actions
    }

    fn render_ssh_entry_row(
        &self,
        ui: &mut egui::Ui,
        entry: &crate::ssh::RemoteEntry,
        actions: &mut SshEntryActions,
    ) {
        let (row_rect, response) = allocate_tree_row(ui);
        paint_hover_highlight(ui, &response, row_rect);
        paint_ssh_entry_name(ui, row_rect, entry, self.settings.font_size, &self.semantic);
        paint_ssh_entry_size(ui, row_rect, entry, self.semantic.tertiary_text);
        if response.clicked() {
            if entry.is_dir {
                actions.navigate_to = Some(entry.path.clone());
            } else {
                actions.open_file = Some(entry.path.clone());
            }
        }
    }

    fn apply_ssh_entry_actions(&mut self, actions: SshEntryActions) {
        if let Some(path) = actions.navigate_to {
            self.ssh_list_dir(&path);
        }
        if let Some(path) = actions.open_file {
            self.ssh_read_file(&path);
        }
    }
}

fn render_ssh_spinner(ui: &mut egui::Ui, label: &str) {
    ui.add_space(SPACE_SM);
    ui.horizontal(|ui| {
        ui.spinner();
        ui.label(label);
    });
}

fn paint_ssh_entry_name(
    ui: &egui::Ui,
    row_rect: egui::Rect,
    entry: &crate::ssh::RemoteEntry,
    font_size: f32,
    semantic: &crate::settings::SemanticColors,
) {
    let icon_str = if entry.is_dir { "\u{1F4C1} " } else { "" };
    let name_color = if entry.is_dir {
        ui.visuals().text_color()
    } else {
        semantic.secondary_text
    };
    ui.painter().text(
        row_rect.left_center() + egui::vec2(8.0, 0.0),
        egui::Align2::LEFT_CENTER,
        format!("{}{}", icon_str, entry.name),
        egui::FontId::proportional(font_size),
        name_color,
    );
}

fn paint_ssh_entry_size(
    ui: &egui::Ui,
    row_rect: egui::Rect,
    entry: &crate::ssh::RemoteEntry,
    color: egui::Color32,
) {
    if entry.is_dir {
        return;
    }
    ui.painter().text(
        egui::pos2(row_rect.right() - 8.0, row_rect.center().y),
        egui::Align2::RIGHT_CENTER,
        format_remote_size(entry.size),
        egui::FontId::monospace(9.0),
        color,
    );
}

fn parent_path(path: &str) -> Option<String> {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() || trimmed == "/" {
        return None;
    }
    match trimmed.rfind('/') {
        Some(0) => Some("/".into()),
        Some(pos) => Some(trimmed[..pos].to_string()),
        None => None,
    }
}

fn format_remote_size(bytes: u64) -> String {
    if bytes == 0 {
        return String::new();
    }
    let kb = bytes as f64 / 1024.0;
    if kb >= 1024.0 {
        format!("{:.1}M", kb / 1024.0)
    } else if kb >= 1.0 {
        format!("{:.0}K", kb)
    } else {
        format!("{}B", bytes)
    }
}
