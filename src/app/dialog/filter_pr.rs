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
                // Page tabs
                ui.horizontal(|ui| {
                    let findings_label = format!("Findings ({})", total);
                    if ui
                        .selectable_label(!self.git.pr_filter_patterns_page, &findings_label)
                        .clicked()
                    {
                        self.git.pr_filter_patterns_page = false;
                    }
                    let patterns_label =
                        format!("Ignore Patterns ({})", self.git.pr_filter_patterns.len());
                    if ui
                        .selectable_label(self.git.pr_filter_patterns_page, &patterns_label)
                        .clicked()
                    {
                        self.git.pr_filter_patterns_page = true;
                    }
                });
                ui.separator();

                if self.git.pr_filter_patterns_page {
                    self.render_patterns_page(ui);
                } else {
                    self.render_findings_page(
                        ui,
                        total,
                        included,
                        excluded,
                        &mut do_import,
                        &mut dismiss,
                    );
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

    fn render_findings_page(
        &mut self,
        ui: &mut egui::Ui,
        total: usize,
        included: usize,
        excluded: usize,
        do_import: &mut bool,
        dismiss: &mut bool,
    ) {
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
        let mut create_pattern_from: Option<(String, String)> = None;

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

                                    // "Ignore" button to create a pattern from this finding
                                    if is_excluded {
                                        let ignore_resp = ui
                                            .button(
                                                egui::RichText::new("Ignore")
                                                    .small()
                                                    .color(self.semantic.tertiary_text),
                                            )
                                            .on_hover_text(
                                                "Create a pattern to auto-exclude similar findings",
                                            );
                                        if ignore_resp.clicked() {
                                            // Pre-fill pattern: use first 80 chars of text
                                            let snippet = if text.len() > 80 {
                                                let end = text
                                                    .char_indices()
                                                    .nth(80)
                                                    .map(|(i, _)| i)
                                                    .unwrap_or(text.len());
                                                text[..end].to_string()
                                            } else {
                                                text.clone()
                                            };
                                            create_pattern_from =
                                                Some((snippet, "text".to_string()));
                                        }
                                    }
                                });
                            });
                    });
                }
            });

        // Handle deferred pattern creation (outside scroll area / borrow scope)
        if let Some((snippet, field)) = create_pattern_from {
            self.git.new_pattern_text = snippet;
            self.git.new_pattern_field = field;
            self.git.pr_filter_patterns_page = true;
        }

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
                *do_import = true;
            }

            if ui.button("Exclude All").clicked() {
                for i in 0..self.git.pr_findings_pending.len() {
                    self.git.pr_findings_excluded.insert(i);
                }
            }
            if ui.button("Include All").clicked() {
                self.git.pr_findings_excluded.clear();
            }

            if ui.button("Cancel").clicked() {
                *dismiss = true;
            }
        });

        // Enter key shortcut for import
        if included > 0 && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            *do_import = true;
        }
    }

    fn render_patterns_page(&mut self, ui: &mut egui::Ui) {
        ui.label("Patterns auto-exclude matching PR findings on import.");
        ui.add_space(SPACE_XS);

        // Add new pattern form
        ui.horizontal(|ui| {
            ui.label("Pattern:");
            let te = egui::TextEdit::singleline(&mut self.git.new_pattern_text)
                .desired_width(280.0)
                .hint_text("substring to match…");
            ui.add(te);
            ui.label("in");
            egui::ComboBox::from_id_salt("new_pattern_field")
                .selected_text(match self.git.new_pattern_field.as_str() {
                    "file_path" => "File path",
                    _ => "Text",
                })
                .width(80.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.git.new_pattern_field, "text".into(), "Text");
                    ui.selectable_value(
                        &mut self.git.new_pattern_field,
                        "file_path".into(),
                        "File path",
                    );
                });
            let can_add = !self.git.new_pattern_text.trim().is_empty();
            if ui.add_enabled(can_add, egui::Button::new("Add")).clicked() {
                let pat = self.git.new_pattern_text.trim().to_string();
                let field = self.git.new_pattern_field.clone();
                if let Ok(id) = self.db.insert_pr_filter_pattern(&pat, &field) {
                    self.git
                        .pr_filter_patterns
                        .push(crate::db::PrFilterPattern {
                            id,
                            pattern: pat,
                            match_field: field,
                        });
                    self.git.new_pattern_text.clear();
                    self.reapply_pr_filter_patterns();
                }
            }
        });
        ui.add_space(SPACE_SM);

        // Existing patterns list
        let card_bg = if self.semantic.is_dark() {
            egui::Color32::from_gray(50)
        } else {
            egui::Color32::from_gray(235)
        };

        let available = ui.available_height() - 10.0;
        let mut delete_id: Option<i64> = None;
        let mut save_edit: Option<(i64, String, String)> = None;

        egui::ScrollArea::vertical()
            .max_height(available.max(100.0))
            .show(ui, |ui| {
                if self.git.pr_filter_patterns.is_empty() {
                    ui.label(
                        egui::RichText::new("No patterns yet. Add one above or click \"Ignore\" on an excluded finding.")
                            .small()
                            .color(self.semantic.tertiary_text),
                    );
                }

                let patterns: Vec<(i64, String, String)> = self
                    .git
                    .pr_filter_patterns
                    .iter()
                    .map(|p| (p.id, p.pattern.clone(), p.match_field.clone()))
                    .collect();

                for (id, pattern, match_field) in patterns {
                    ui.push_id(id, |ui| {
                        egui::Frame::new()
                            .fill(card_bg)
                            .corner_radius(4.0)
                            .inner_margin(6.0)
                            .outer_margin(egui::Margin::symmetric(0, 2))
                            .show(ui, |ui| {
                                let is_editing = self
                                    .git
                                    .editing_pattern
                                    .as_ref()
                                    .map_or(false, |(eid, _, _)| *eid == id);

                                if is_editing {
                                    // Extract editing state to avoid borrow conflicts
                                    let mut edit_text = self.git.editing_pattern.as_ref().unwrap().1.clone();
                                    let mut edit_field = self.git.editing_pattern.as_ref().unwrap().2.clone();
                                    let mut cancel_edit = false;
                                    ui.horizontal(|ui| {
                                        let te =
                                            egui::TextEdit::singleline(&mut edit_text)
                                                .desired_width(280.0);
                                        ui.add(te);
                                        egui::ComboBox::from_id_salt(format!("edit_field_{}", id))
                                            .selected_text(match edit_field.as_str() {
                                                "file_path" => "File path",
                                                _ => "Text",
                                            })
                                            .width(80.0)
                                            .show_ui(ui, |ui| {
                                                ui.selectable_value(
                                                    &mut edit_field,
                                                    "text".into(),
                                                    "Text",
                                                );
                                                ui.selectable_value(
                                                    &mut edit_field,
                                                    "file_path".into(),
                                                    "File path",
                                                );
                                            });
                                        if ui.button("Save").clicked() {
                                            save_edit = Some((
                                                id,
                                                edit_text.clone(),
                                                edit_field.clone(),
                                            ));
                                        }
                                        if ui.button("Cancel").clicked() {
                                            cancel_edit = true;
                                        }
                                    });
                                    // Write back edited values
                                    if cancel_edit {
                                        self.git.editing_pattern = None;
                                    } else if self.git.editing_pattern.is_some() {
                                        self.git.editing_pattern = Some((id, edit_text, edit_field));
                                    }
                                } else {
                                    ui.horizontal(|ui| {
                                        let field_label = match match_field.as_str() {
                                            "file_path" => "file path",
                                            _ => "text",
                                        };
                                        ui.label(
                                            egui::RichText::new(format!(
                                                "\"{}\" in {}",
                                                pattern, field_label
                                            ))
                                            .small(),
                                        );
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if ui
                                                    .button(
                                                        egui::RichText::new("\u{2716}")
                                                            .small()
                                                            .color(self.semantic.danger),
                                                    )
                                                    .on_hover_text("Delete pattern")
                                                    .clicked()
                                                {
                                                    delete_id = Some(id);
                                                }
                                                if ui
                                                    .button(
                                                        egui::RichText::new("\u{270E}")
                                                            .small(),
                                                    )
                                                    .on_hover_text("Edit pattern")
                                                    .clicked()
                                                {
                                                    self.git.editing_pattern = Some((
                                                        id,
                                                        pattern.clone(),
                                                        match_field.clone(),
                                                    ));
                                                }
                                            },
                                        );
                                    });
                                }
                            });
                    });
                }
            });

        // Handle deferred mutations
        if let Some(id) = delete_id {
            match self.db.delete_pr_filter_pattern(id) {
                Ok(()) => {
                    self.git.pr_filter_patterns.retain(|p| p.id != id);
                    self.reapply_pr_filter_patterns();
                }
                Err(e) => {
                    eprintln!("Failed to delete PR filter pattern {}: {}", id, e);
                    self.set_status_message(format!("Failed to delete filter pattern: {}", e));
                }
            }
        }
        if let Some((id, text, field)) = save_edit {
            let trimmed = text.trim().to_string();
            if !trimmed.is_empty() {
                match self.db.update_pr_filter_pattern(id, &trimmed, &field) {
                    Ok(()) => {
                        if let Some(p) = self.git.pr_filter_patterns.iter_mut().find(|p| p.id == id)
                        {
                            p.pattern = trimmed;
                            p.match_field = field;
                        }
                        self.reapply_pr_filter_patterns();
                        self.git.editing_pattern = None;
                    }
                    Err(e) => {
                        eprintln!("Failed to update PR filter pattern {}: {}", id, e);
                        self.set_status_message(format!("Failed to update filter pattern: {}", e));
                    }
                }
            } else {
                self.git.editing_pattern = None;
            }
        }
    }

    /// Re-apply all patterns to the pending findings, updating the excluded set.
    fn reapply_pr_filter_patterns(&mut self) {
        self.git.pr_findings_excluded.clear();
        for (idx, finding) in self.git.pr_findings_pending.iter().enumerate() {
            for pat in &self.git.pr_filter_patterns {
                let haystack = match pat.match_field.as_str() {
                    "file_path" => &finding.file_path,
                    _ => &finding.text,
                };
                if haystack
                    .to_lowercase()
                    .contains(&pat.pattern.to_lowercase())
                {
                    self.git.pr_findings_excluded.insert(idx);
                    break;
                }
            }
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
