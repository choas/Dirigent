use eframe::egui;

use super::types::CodeLineActions;
use crate::app::{icon, DirigentApp};

/// Render the cue input row (images + text field) after the last selected line.
pub(crate) fn render_cue_input(
    ui: &mut egui::Ui,
    app: &mut DirigentApp,
    active_idx: usize,
    sel_start: Option<usize>,
    sel_end: Option<usize>,
    actions: &mut CodeLineActions,
) {
    let range_label = if sel_start == sel_end {
        format!("L{}", sel_start.unwrap_or(0))
    } else {
        format!("L{}-{}", sel_start.unwrap_or(0), sel_end.unwrap_or(0))
    };

    render_cue_images_row(ui, app, active_idx);
    render_cue_text_input(ui, app, active_idx, &range_label, actions);
}

/// Render attached cue images row (if any).
fn render_cue_images_row(ui: &mut egui::Ui, app: &mut DirigentApp, active_idx: usize) {
    if app.viewer.tabs[active_idx].cue_images.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.label("     ");
        ui.label(
            egui::RichText::new("Images:")
                .small()
                .color(app.semantic.accent),
        );
        let mut remove_idx = None;
        for (i, path) in app.viewer.tabs[active_idx].cue_images.iter().enumerate() {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string_lossy().to_string());
            ui.label(egui::RichText::new(&name).monospace().small());
            if ui.small_button("\u{2715}").clicked() {
                remove_idx = Some(i);
            }
        }
        if let Some(i) = remove_idx {
            app.viewer.tabs[active_idx].cue_images.remove(i);
        }
    });
}

/// Render the cue text input field with Add/Close buttons.
fn render_cue_text_input(
    ui: &mut egui::Ui,
    app: &mut DirigentApp,
    active_idx: usize,
    range_label: &str,
    actions: &mut CodeLineActions,
) {
    ui.horizontal(|ui| {
        ui.label("     ");
        ui.label(
            egui::RichText::new(range_label)
                .monospace()
                .color(app.semantic.success),
        );
        if ui.button("+").on_hover_text("Attach images").clicked() {
            if let Some(paths) = rfd::FileDialog::new()
                .add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp", "bmp"])
                .pick_files()
            {
                app.viewer.tabs[active_idx].cue_images.extend(paths);
            }
        }
        let input_response = ui.add(
            egui::TextEdit::singleline(&mut app.viewer.tabs[active_idx].cue_input)
                .desired_width(ui.available_width() - 80.0)
                .hint_text("Add a cue...")
                .font(egui::TextStyle::Monospace),
        );
        let enter = input_response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if ui.button("Add").clicked() || enter {
            actions.submit_cue = true;
        }
        let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if ui
            .button(icon("\u{2715}", app.settings.font_size))
            .clicked()
            || esc
        {
            actions.clear_selection = true;
        }
    });
}
