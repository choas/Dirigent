use eframe::egui;

use super::super::mermaid::{self, MermaidState};
use super::super::DirigentApp;

impl DirigentApp {
    /// Render the enlarged Mermaid diagram viewer: zoomable image plus PNG/SVG
    /// export. Opened by clicking a rendered diagram in the Markdown viewer.
    pub(in super::super) fn render_mermaid_dialog(&mut self, ctx: &egui::Context) {
        if self.mermaid_dialog.is_none() {
            return;
        }

        let (source, dark) = {
            let dialog = self.mermaid_dialog.as_ref().expect("checked above");
            (dialog.source.clone(), dialog.dark)
        };

        // Resolve the cached texture (clone the handle so we can drop the borrow).
        let texture = match self.mermaid.get(&source, dark) {
            Some(MermaidState::Ready(t)) => Some(t.clone()),
            _ => None,
        };

        let mut open = true;
        let mut export: Option<&'static str> = None;

        {
            let dialog = self.mermaid_dialog.as_mut().expect("checked above");
            egui::Window::new("Mermaid Diagram")
                .open(&mut open)
                .default_size([720.0, 540.0])
                .resizable(true)
                .collapsible(false)
                .frame(self.semantic.dialog_frame())
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        if ui.button("\u{2212}").on_hover_text("Zoom out").clicked() {
                            dialog.zoom = (dialog.zoom / 1.25).max(0.1);
                        }
                        ui.label(format!("{}%", (dialog.zoom * 100.0) as u32));
                        if ui.button("+").on_hover_text("Zoom in").clicked() {
                            dialog.zoom = (dialog.zoom * 1.25).min(10.0);
                        }
                        if ui.button("Fit").on_hover_text("Reset zoom").clicked() {
                            dialog.zoom = 1.0;
                        }
                        ui.separator();
                        if ui
                            .button("Download PNG")
                            .on_hover_text("Export the diagram as a PNG image")
                            .clicked()
                        {
                            export = Some("png");
                        }
                        if ui
                            .button("Download SVG")
                            .on_hover_text("Export the diagram as an SVG image")
                            .clicked()
                        {
                            export = Some("svg");
                        }
                    });
                    ui.separator();

                    // Cmd/Ctrl + scroll to zoom.
                    let scroll = ui.input(|i| {
                        if i.modifiers.command {
                            i.smooth_scroll_delta.y
                        } else {
                            0.0
                        }
                    });
                    if scroll != 0.0 {
                        let factor = if scroll > 0.0 { 1.1 } else { 1.0 / 1.1 };
                        dialog.zoom = (dialog.zoom * factor).clamp(0.1, 10.0);
                    }

                    egui::ScrollArea::both().auto_shrink([false; 2]).show(
                        ui,
                        |ui| match &texture {
                            Some(tex) => {
                                let size = tex.size_vec2() * dialog.zoom;
                                ui.add(egui::Image::new((tex.id(), size)));
                            }
                            None => {
                                ui.label("Diagram is not available.");
                            }
                        },
                    );
                });
        }

        if let Some(format) = export {
            self.export_mermaid(&source, dark, format);
        }
        if !open {
            self.mermaid_dialog = None;
        }
    }

    /// Render the diagram to the chosen format and save it via a file dialog.
    fn export_mermaid(&mut self, source: &str, dark: bool, format: &str) {
        let file_name = format!("diagram.{format}");
        let Some(path) = rfd::FileDialog::new()
            .set_title("Export Diagram")
            .set_file_name(&file_name)
            .add_filter(format.to_uppercase(), &[format])
            .save_file()
        else {
            return;
        };

        match mermaid::render_to_bytes(source, dark, format) {
            Ok(bytes) => match std::fs::write(&path, &bytes) {
                Ok(()) => {
                    self.set_status_message(format!("Diagram exported to {}", path.display()))
                }
                Err(e) => self.set_status_message(format!("Failed to write diagram: {e}")),
            },
            Err(e) => self.set_status_message(format!("Failed to render diagram: {e}")),
        }
    }
}
