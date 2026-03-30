use eframe::egui;

use super::super::{icon, DirigentApp, SPACE_SM, SPACE_XS};

impl DirigentApp {
    pub(in crate::app) fn render_filter_pr_dialog(&mut self, ctx: &egui::Context) {
        if !self.git.show_pr_filter {
            return;
        }

        let total = self.git.pr_findings_pending.len();
        let excluded = self.git.pr_findings_excluded.len();
        let included = total - excluded;

        let card_bg = if self.semantic.is_dark() {
            egui::Color32::from_gray(50)
        } else {
            egui::Color32::from_gray(235)
        };
        let card_bg_excluded = if self.semantic.is_dark() {
            egui::Color32::from_gray(30)
        } else {
            egui::Color32::from_gray(220)
        };

        // Track actions via self fields to avoid closure capture issues
        let mut do_import = false;
        let mut dismiss = false;

        egui::Window::new("Filter PR Findings")
            .collapsible(false)
            .resizable(true)
            .default_width(600.0)
            .default_height(500.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.dialog_frame())
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.label(format!(
                    "Review {} new findings from PR #{}. Exclude items you don't want imported.",
                    total,
                    self.git.import_pr_number.trim()
                ));
                ui.add_space(SPACE_XS);

                ui.label(
                    egui::RichText::new(format!("{} included, {} excluded", included, excluded))
                        .small()
                        .color(self.semantic.tertiary_text),
                );
                ui.add_space(SPACE_SM);

                // Scrollable list of findings
                let available = ui.available_height() - 50.0;
                egui::ScrollArea::vertical()
                    .max_height(available.max(150.0))
                    .show(ui, |ui| {
                        let findings: Vec<(usize, String, String, usize)> = self
                            .git
                            .pr_findings_pending
                            .iter()
                            .enumerate()
                            .map(|(i, f)| (i, f.file_path.clone(), f.text.clone(), f.line_number))
                            .collect();

                        for (idx, file_path, text, line_number) in findings {
                            let is_excluded = self.git.pr_findings_excluded.contains(&idx);
                            ui.push_id(idx, |ui| {
                                let fill = if is_excluded {
                                    card_bg_excluded
                                } else {
                                    card_bg
                                };
                                egui::Frame::new()
                                    .fill(fill)
                                    .corner_radius(4.0)
                                    .inner_margin(6.0)
                                    .outer_margin(egui::Margin::symmetric(0, 2))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            if is_excluded {
                                                if ui
                                                    .button(
                                                        egui::RichText::new("\u{2795}")
                                                            .color(self.semantic.success),
                                                    )
                                                    .on_hover_text("Include this finding")
                                                    .clicked()
                                                {
                                                    self.git.pr_findings_excluded.remove(&idx);
                                                }
                                            } else if ui
                                                .button(
                                                    egui::RichText::new("\u{2796}")
                                                        .color(self.semantic.danger),
                                                )
                                                .on_hover_text("Exclude this finding")
                                                .clicked()
                                            {
                                                self.git.pr_findings_excluded.insert(idx);
                                            }

                                            ui.vertical(|ui| {
                                                if !file_path.is_empty() {
                                                    let loc = if line_number > 0 {
                                                        format!("{}:{}", file_path, line_number)
                                                    } else {
                                                        file_path.to_string()
                                                    };
                                                    ui.label(
                                                        egui::RichText::new(loc)
                                                            .small()
                                                            .strong()
                                                            .color(self.semantic.accent),
                                                    );
                                                }
                                                let display_text = if text.len() > 200 {
                                                    let end = text
                                                        .char_indices()
                                                        .nth(200)
                                                        .map(|(i, _)| i)
                                                        .unwrap_or(text.len());
                                                    format!("{}…", &text[..end])
                                                } else {
                                                    text.to_string()
                                                };
                                                let text_color = if is_excluded {
                                                    self.semantic.tertiary_text
                                                } else {
                                                    self.semantic.secondary_text
                                                };
                                                ui.label(
                                                    egui::RichText::new(display_text)
                                                        .small()
                                                        .color(text_color),
                                                );
                                            });
                                        });
                                    });
                            });
                        }
                    });

                ui.add_space(SPACE_SM);

                let fs = self.settings.font_size;
                ui.horizontal(|ui| {
                    let import_label = format!("\u{2193} Import {} Findings", included);
                    let import_btn =
                        egui::Button::new(icon(&import_label, fs).color(self.semantic.badge_text))
                            .fill(self.semantic.accent);
                    if ui
                        .add_enabled(included > 0, import_btn)
                        .on_hover_text("Import selected findings to Inbox")
                        .clicked()
                    {
                        do_import = true;
                    }

                    if ui.button("Exclude All").clicked() {
                        for i in 0..total {
                            self.git.pr_findings_excluded.insert(i);
                        }
                    }
                    if ui.button("Include All").clicked() {
                        self.git.pr_findings_excluded.clear();
                    }

                    if ui.button("Cancel").clicked() {
                        dismiss = true;
                    }
                });

                // Enter key shortcut for import
                if included > 0 && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    do_import = true;
                }
            });

        if do_import {
            self.import_filtered_pr_findings();
        } else if dismiss {
            self.git.show_pr_filter = false;
            self.git.pr_findings_pending.clear();
            self.git.pr_findings_excluded.clear();
        }
    }

    fn import_filtered_pr_findings(&mut self) {
        let findings: Vec<crate::sources::PrFinding> = self
            .git
            .pr_findings_pending
            .iter()
            .enumerate()
            .filter(|(i, _)| !self.git.pr_findings_excluded.contains(i))
            .map(|(_, f)| f.clone())
            .collect();

        // Close dialogs and clear state
        self.git.show_pr_filter = false;
        self.git.show_import_pr = false;
        self.git.pr_findings_pending.clear();
        self.git.pr_findings_excluded.clear();

        // Clear source filter so newly imported cues are visible in the pool
        self.sources.filter = None;

        self.handle_pr_findings(findings);
    }
}
