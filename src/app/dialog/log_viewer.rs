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
                ui.horizontal(|ui| {
                    ui.label("Filter:");
                    for (label, filter) in [
                        ("All", log::LevelFilter::Trace),
                        ("Debug", log::LevelFilter::Debug),
                        ("Info", log::LevelFilter::Info),
                        ("Warn", log::LevelFilter::Warn),
                        ("Error", log::LevelFilter::Error),
                    ] {
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
                ui.separator();

                let entries = crate::log_collector::entries_snapshot();

                let row_height = ui.text_style_height(&egui::TextStyle::Monospace) + 2.0;
                let filtered: Vec<_> = entries
                    .iter()
                    .filter(|(level, _, _, _)| *level <= self.log_viewer_filter)
                    .collect();

                let num_rows = filtered.len();

                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .stick_to_bottom(self.log_viewer_auto_scroll)
                    .show_rows(ui, row_height, num_rows, |ui, range| {
                        ui.style_mut().override_font_id = Some(egui::FontId::monospace(
                            ui.text_style_height(&egui::TextStyle::Body) * 0.85,
                        ));
                        for &(level, ref target, ref msg, elapsed) in
                            filtered[range].iter().copied()
                        {
                            ui.horizontal(|ui| {
                                let ts = format!("{:>8.3}s", elapsed);
                                ui.label(
                                    egui::RichText::new(ts).color(self.semantic.tertiary_text),
                                );

                                let (level_str, color) = match level {
                                    Level::Error => ("ERROR", self.semantic.danger),
                                    Level::Warn => (" WARN", self.semantic.warning),
                                    Level::Info => (" INFO", self.semantic.accent),
                                    Level::Debug => ("DEBUG", self.semantic.tertiary_text),
                                    Level::Trace => ("TRACE", self.semantic.tertiary_text),
                                };
                                ui.label(egui::RichText::new(level_str).color(color));

                                let short_target = target.rsplit("::").next().unwrap_or(target);
                                ui.label(
                                    egui::RichText::new(short_target)
                                        .color(self.semantic.secondary_text),
                                );

                                ui.label(msg);
                            });
                        }
                    });
            });
        if !open {
            self.show_log_viewer = false;
        }
    }
}
