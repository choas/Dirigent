use eframe::egui;

use super::super::DirigentApp;

#[derive(Default)]
pub(super) struct ErrorDialogActions {
    pub(super) dismiss: bool,
    pub(super) create_cue: bool,
    pub(super) try_again: bool,
}

pub(super) struct ErrorDialogSpec<'a> {
    pub(super) title: &'a str,
    pub(super) intro: &'a str,
    pub(super) error_message: &'a str,
    pub(super) error_max_height: f32,
    pub(super) create_button: &'a str,
    pub(super) create_tooltip: &'a str,
    pub(super) try_again_tooltip: Option<&'a str>,
    pub(super) suggestion: Option<&'a str>,
    pub(super) suggestion_color: egui::Color32,
}

pub(super) fn render_error_dialog(
    ctx: &egui::Context,
    frame: egui::Frame,
    spec: ErrorDialogSpec<'_>,
) -> ErrorDialogActions {
    let mut actions = ErrorDialogActions::default();

    egui::Window::new(spec.title)
        .collapsible(false)
        .resizable(true)
        .default_width(480.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .frame(frame)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            ui.label(spec.intro);
            ui.add_space(8.0);
            render_error_text(ui, spec.error_message, spec.error_max_height);
            render_suggestion(ui, spec.suggestion, spec.suggestion_color);
            render_error_buttons(ui, &spec, &mut actions);
        });

    actions
}

fn render_error_text(ui: &mut egui::Ui, message: &str, max_height: f32) {
    egui::ScrollArea::vertical()
        .max_height(max_height)
        .show(ui, |ui| {
            let mut error_text = message.to_string();
            ui.add(
                egui::TextEdit::multiline(&mut error_text)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY),
            );
        });
}

fn render_suggestion(ui: &mut egui::Ui, suggestion: Option<&str>, color: egui::Color32) {
    let Some(text) = suggestion else {
        ui.add_space(12.0);
        return;
    };
    ui.add_space(10.0);
    ui.label(egui::RichText::new("Suggestion").strong());
    ui.add_space(2.0);
    ui.label(egui::RichText::new(text).color(color));
    ui.add_space(12.0);
}

fn render_error_buttons(
    ui: &mut egui::Ui,
    spec: &ErrorDialogSpec<'_>,
    actions: &mut ErrorDialogActions,
) {
    ui.horizontal(|ui| {
        if let Some(tooltip) = spec.try_again_tooltip {
            actions.try_again = ui.button("Try Again").on_hover_text(tooltip).clicked();
        }
        actions.create_cue = ui
            .button(spec.create_button)
            .on_hover_text(spec.create_tooltip)
            .clicked();
        actions.dismiss = ui.button("Dismiss").clicked();
    });
}

pub(super) fn create_global_error_cue(
    app: &mut DirigentApp,
    cue_text: &str,
    success_message: &str,
) {
    match app.db.insert_global_cue(cue_text) {
        Ok(_id) => {
            app.reload_cues();
            app.set_status_message(success_message.to_string());
        }
        Err(e) => {
            app.set_status_message(format!("Failed to create cue: {}", e));
        }
    }
}
