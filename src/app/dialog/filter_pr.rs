use eframe::egui;

use super::super::{icon, DirigentApp, SPACE_SM, SPACE_XS};

/// Truncate `text` to `max` characters. If truncated and `ellipsis` is true, append "…".
fn truncate(text: &str, max: usize, ellipsis: bool) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let end = text
        .char_indices()
        .nth(max)
        .map(|(i, _)| i)
        .unwrap_or(text.len());
    if ellipsis {
        format!("{}…", &text[..end])
    } else {
        text[..end].to_string()
    }
}

/// Format a file location string, returning `None` if the path is empty.
fn format_location(file_path: &str, line_number: usize) -> Option<String> {
    if file_path.is_empty() {
        return None;
    }
    if line_number > 0 {
        Some(format!("{}:{}", file_path, line_number))
    } else {
        Some(file_path.to_string())
    }
}

/// Render a combo box for selecting the match field ("text" or "file_path").
fn field_combo(ui: &mut egui::Ui, salt: &str, field: &mut String) {
    egui::ComboBox::from_id_salt(salt)
        .selected_text(match field.as_str() {
            "file_path" => "File path",
            _ => "Text",
        })
        .width(80.0)
        .show_ui(ui, |ui| {
            ui.selectable_value(field, "text".into(), "Text");
            ui.selectable_value(field, "file_path".into(), "File path");
        });
}

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

        let create_pattern_from = self.render_findings_list(ui);

        if let Some((snippet, field)) = create_pattern_from {
            self.git.new_pattern_text = snippet;
            self.git.new_pattern_field = field;
            self.git.pr_filter_patterns_page = true;
        }

        ui.add_space(SPACE_SM);
        self.render_findings_action_bar(ui, included, do_import, dismiss);

        if included > 0 && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            *do_import = true;
        }
    }

    fn card_backgrounds(&self) -> (egui::Color32, egui::Color32) {
        if self.semantic.is_dark() {
            (egui::Color32::from_gray(50), egui::Color32::from_gray(30))
        } else {
            (egui::Color32::from_gray(235), egui::Color32::from_gray(220))
        }
    }

    fn render_findings_list(&mut self, ui: &mut egui::Ui) -> Option<(String, String)> {
        let available = ui.available_height() - 50.0;
        let mut create_pattern_from: Option<(String, String)> = None;

        let findings: Vec<(usize, crate::sources::PrFinding)> = self
            .git
            .pr_findings_pending
            .iter()
            .enumerate()
            .map(|(i, f)| (i, f.clone()))
            .collect();

        egui::ScrollArea::vertical()
            .max_height(available.max(150.0))
            .show(ui, |ui| {
                for (idx, finding) in &findings {
                    if let Some(pattern) = self.render_finding_card(ui, *idx, finding) {
                        create_pattern_from = Some(pattern);
                    }
                }
            });

        create_pattern_from
    }

    fn render_finding_card(
        &mut self,
        ui: &mut egui::Ui,
        idx: usize,
        finding: &crate::sources::PrFinding,
    ) -> Option<(String, String)> {
        let is_excluded = self.git.pr_findings_excluded.contains(&idx);
        let (card_bg, card_bg_excluded) = self.card_backgrounds();
        let fill = if is_excluded {
            card_bg_excluded
        } else {
            card_bg
        };

        let mut result = None;
        ui.push_id(idx, |ui| {
            egui::Frame::new()
                .fill(fill)
                .corner_radius(4.0)
                .inner_margin(6.0)
                .outer_margin(egui::Margin::symmetric(0, 2))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        self.render_finding_toggle(ui, idx, is_excluded);
                        self.render_finding_body(
                            ui,
                            &finding.file_path,
                            &finding.text,
                            finding.line_number,
                            is_excluded,
                        );
                        if is_excluded {
                            result = self.render_ignore_button(ui, &finding.text);
                        }
                    });
                });
        });
        result
    }

    fn render_finding_toggle(&mut self, ui: &mut egui::Ui, idx: usize, is_excluded: bool) {
        if is_excluded {
            if ui
                .button(egui::RichText::new("\u{2795}").color(self.semantic.success))
                .on_hover_text("Include this finding")
                .clicked()
            {
                self.git.pr_findings_excluded.remove(&idx);
            }
        } else if ui
            .button(egui::RichText::new("\u{2796}").color(self.semantic.danger))
            .on_hover_text("Exclude this finding")
            .clicked()
        {
            self.git.pr_findings_excluded.insert(idx);
        }
    }

    fn render_finding_body(
        &self,
        ui: &mut egui::Ui,
        file_path: &str,
        text: &str,
        line_number: usize,
        is_excluded: bool,
    ) {
        let loc = format_location(file_path, line_number);
        let display_text = truncate(text, 200, true);
        let text_color = if is_excluded {
            self.semantic.tertiary_text
        } else {
            self.semantic.secondary_text
        };

        ui.vertical(|ui| {
            if let Some(loc) = &loc {
                ui.label(
                    egui::RichText::new(loc)
                        .small()
                        .strong()
                        .color(self.semantic.accent),
                );
            }
            ui.label(egui::RichText::new(display_text).small().color(text_color));
        });
    }

    fn render_ignore_button(&self, ui: &mut egui::Ui, text: &str) -> Option<(String, String)> {
        let resp = ui
            .button(
                egui::RichText::new("Ignore")
                    .small()
                    .color(self.semantic.tertiary_text),
            )
            .on_hover_text("Create a pattern to auto-exclude similar findings");
        if resp.clicked() {
            let snippet = truncate(text, 80, false);
            return Some((snippet, "text".to_string()));
        }
        None
    }

    fn render_findings_action_bar(
        &mut self,
        ui: &mut egui::Ui,
        included: usize,
        do_import: &mut bool,
        dismiss: &mut bool,
    ) {
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
    }

    fn render_patterns_page(&mut self, ui: &mut egui::Ui) {
        ui.label("Patterns auto-exclude matching PR findings on import.");
        ui.add_space(SPACE_XS);

        self.render_add_pattern_form(ui);
        ui.add_space(SPACE_SM);

        let (delete_id, save_edit) = self.render_pattern_list(ui);

        if let Some(id) = delete_id {
            self.handle_pattern_delete(id);
        }
        if let Some((id, text, field)) = save_edit {
            self.handle_pattern_save(id, text, field);
        }
    }

    fn render_add_pattern_form(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Pattern:");
            let te = egui::TextEdit::singleline(&mut self.git.new_pattern_text)
                .desired_width(280.0)
                .hint_text("substring to match…");
            ui.add(te);
            ui.label("in");
            field_combo(ui, "new_pattern_field", &mut self.git.new_pattern_field);
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
    }

    fn render_pattern_list(
        &mut self,
        ui: &mut egui::Ui,
    ) -> (Option<i64>, Option<(i64, String, String)>) {
        let (card_bg, _) = self.card_backgrounds();
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
                    return;
                }

                let patterns: Vec<(i64, String, String)> = self
                    .git
                    .pr_filter_patterns
                    .iter()
                    .map(|p| (p.id, p.pattern.clone(), p.match_field.clone()))
                    .collect();

                for (id, pattern, match_field) in patterns {
                    let (del, save) =
                        self.render_pattern_card(ui, id, &pattern, &match_field, card_bg);
                    if del {
                        delete_id = Some(id);
                    }
                    if let Some(s) = save {
                        save_edit = Some(s);
                    }
                }
            });

        (delete_id, save_edit)
    }

    fn render_pattern_card(
        &mut self,
        ui: &mut egui::Ui,
        id: i64,
        pattern: &str,
        match_field: &str,
        card_bg: egui::Color32,
    ) -> (bool, Option<(i64, String, String)>) {
        let mut delete = false;
        let mut save = None;

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
                        .is_some_and(|(eid, _, _)| *eid == id);

                    if is_editing {
                        save = self.render_editing_pattern(ui, id);
                    } else {
                        delete = self.render_display_pattern(ui, id, pattern, match_field);
                    }
                });
        });

        (delete, save)
    }

    fn render_editing_pattern(
        &mut self,
        ui: &mut egui::Ui,
        id: i64,
    ) -> Option<(i64, String, String)> {
        let (_, text, field) = self.git.editing_pattern.as_ref()?;
        let mut edit_text = text.clone();
        let mut edit_field = field.clone();
        let mut cancel = false;
        let mut save = None;

        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut edit_text).desired_width(280.0));
            field_combo(ui, &format!("edit_field_{}", id), &mut edit_field);
            if ui.button("Save").clicked() {
                save = Some((id, edit_text.clone(), edit_field.clone()));
            }
            if ui.button("Cancel").clicked() {
                cancel = true;
            }
        });

        if cancel {
            self.git.editing_pattern = None;
        } else {
            self.git.editing_pattern = Some((id, edit_text, edit_field));
        }
        save
    }

    fn render_display_pattern(
        &mut self,
        ui: &mut egui::Ui,
        id: i64,
        pattern: &str,
        match_field: &str,
    ) -> bool {
        let mut delete = false;
        ui.horizontal(|ui| {
            let field_label = match match_field {
                "file_path" => "file path",
                _ => "text",
            };
            ui.label(egui::RichText::new(format!("\"{}\" in {}", pattern, field_label)).small());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button(
                        egui::RichText::new("\u{2716}")
                            .small()
                            .color(self.semantic.danger),
                    )
                    .on_hover_text("Delete pattern")
                    .clicked()
                {
                    delete = true;
                }
                if ui
                    .button(egui::RichText::new("\u{270E}").small())
                    .on_hover_text("Edit pattern")
                    .clicked()
                {
                    self.git.editing_pattern =
                        Some((id, pattern.to_string(), match_field.to_string()));
                }
            });
        });
        delete
    }

    fn handle_pattern_delete(&mut self, id: i64) {
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

    fn handle_pattern_save(&mut self, id: i64, text: String, field: String) {
        let trimmed = text.trim().to_string();
        if trimmed.is_empty() {
            self.git.editing_pattern = None;
            return;
        }
        match self.db.update_pr_filter_pattern(id, &trimmed, &field) {
            Ok(()) => {
                if let Some(p) = self.git.pr_filter_patterns.iter_mut().find(|p| p.id == id) {
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
