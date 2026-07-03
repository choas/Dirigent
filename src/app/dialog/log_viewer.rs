use eframe::egui;
use log::Level;

use super::super::DirigentApp;

impl DirigentApp {
    pub(in super::super) fn render_log_viewer(&mut self, ctx: &egui::Context) {
        if !self.show_log_viewer {
            return;
        }
        let mut open = true;
        egui::Window::new("Application Logs")
            .open(&mut open)
            .default_size([700.0, 450.0])
            .resizable(true)
            .collapsible(false)
            .show(ctx, |ui| {
                self.render_log_viewer_toolbar(ui);
                ui.separator();

                let entries = crate::log_collector::entries_snapshot();
                let row_height = ui.text_style_height(&egui::TextStyle::Monospace) + 2.0;
                let filtered: Vec<_> = entries
                    .iter()
                    .filter(|(level, _, _, _)| *level <= self.log_viewer_filter)
                    .collect();

                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .stick_to_bottom(self.log_viewer_auto_scroll)
                    .show_rows(ui, row_height, filtered.len(), |ui, range| {
                        self.render_log_rows(ui, &filtered[range]);
                    });
            });
        if !open {
            self.show_log_viewer = false;
        }
    }

    fn render_log_viewer_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Filter:");
            for (label, filter) in log_filter_options() {
                if ui
                    .selectable_label(self.log_viewer_filter == filter, label)
                    .clicked()
                {
                    self.log_viewer_filter = filter;
                }
            }
            ui.separator();
            ui.checkbox(&mut self.log_viewer_auto_scroll, "Auto-scroll");
        });
    }

    fn render_log_rows(&self, ui: &mut egui::Ui, rows: &[&(Level, String, String, f64)]) {
        ui.style_mut().override_font_id = Some(egui::FontId::monospace(
            ui.text_style_height(&egui::TextStyle::Body) * 0.85,
        ));
        for &(level, ref target, ref msg, elapsed) in rows.iter().copied() {
            self.render_log_row(ui, level, target, msg, elapsed);
        }
    }

    fn render_log_row(
        &self,
        ui: &mut egui::Ui,
        level: Level,
        target: &str,
        msg: &str,
        elapsed: f64,
    ) {
        ui.horizontal(|ui| {
            let ts = format!("{:>8.3}s", elapsed);
            ui.label(egui::RichText::new(ts).color(self.semantic.tertiary_text));

            let (level_str, color) = self.log_level_display(level);
            ui.label(egui::RichText::new(level_str).color(color));

            let short_target = target.rsplit("::").next().unwrap_or(target);
            ui.label(egui::RichText::new(short_target).color(self.semantic.secondary_text));

            ui.label(msg);
        });
    }

    fn log_level_display(&self, level: Level) -> (&'static str, egui::Color32) {
        match level {
            Level::Error => ("ERROR", self.semantic.danger),
            Level::Warn => (" WARN", self.semantic.warning),
            Level::Info => (" INFO", self.semantic.accent),
            Level::Debug => ("DEBUG", self.semantic.tertiary_text),
            Level::Trace => ("TRACE", self.semantic.tertiary_text),
        }
    }
}

fn log_filter_options() -> [(&'static str, log::LevelFilter); 5] {
    [
        ("All", log::LevelFilter::Trace),
        ("Debug", log::LevelFilter::Debug),
        ("Info", log::LevelFilter::Info),
        ("Warn", log::LevelFilter::Warn),
        ("Error", log::LevelFilter::Error),
    ]
}
