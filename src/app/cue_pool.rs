use std::collections::{BTreeSet, HashSet};

use eframe::egui;

use super::{icon, CueAction, DirigentApp};
use crate::db::{Cue, CueStatus};
use crate::diff_view::{self, DiffViewMode};
use crate::git;

impl DirigentApp {
    pub(super) fn render_cue_pool(&mut self, ctx: &egui::Context) {
        egui::SidePanel::right("cue_pool")
            .default_width(250.0)
            .min_width(200.0)
            .show(ctx, |ui| {
                // Header: "Cues" heading + "+" playbook button
                let mut selected_play_prompt: Option<String> = None;
                let mut custom_cue_requested = false;
                ui.horizontal(|ui| {
                    ui.heading("Cues");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let plus_btn = ui.button("+").on_hover_text("Playbook");
                        let popup_id = ui.make_persistent_id("playbook_popup");
                        if plus_btn.clicked() {
                            ui.memory_mut(|mem| mem.toggle_popup(popup_id));
                        }
                        egui::popup_below_widget(ui, popup_id, &plus_btn, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                            ui.set_min_width(200.0);
                            ui.label(egui::RichText::new("Playbook").strong());
                            ui.separator();
                            for play in &self.settings.playbook {
                                if ui.selectable_label(false, &play.name).clicked() {
                                    selected_play_prompt = Some(play.prompt.clone());
                                }
                            }
                            if !self.settings.playbook.is_empty() {
                                ui.separator();
                            }
                            if ui.selectable_label(false, "+ Custom cue...").clicked() {
                                custom_cue_requested = true;
                            }
                        });
                    });
                });

                // Handle playbook selection
                if let Some(prompt) = selected_play_prompt {
                    let _ = self.db.insert_cue(&prompt, "", 0, None);
                    self.reload_cues();
                }
                if custom_cue_requested {
                    // Focus the global prompt field by clearing and letting egui pick it up
                    self.global_prompt_input.clear();
                }

                // Source filter dropdown
                let unique_labels: Vec<String> = {
                    let mut labels = BTreeSet::new();
                    for c in &self.cues {
                        if let Some(ref label) = c.source_label {
                            labels.insert(label.clone());
                        }
                    }
                    for s in &self.settings.sources {
                        if s.enabled {
                            labels.insert(s.label.clone());
                        }
                    }
                    labels.into_iter().collect()
                };

                if !unique_labels.is_empty() {
                    ui.horizontal(|ui| {
                        let current = self.source_filter.as_deref().unwrap_or("All");
                        egui::ComboBox::from_id_salt("source_filter")
                            .selected_text(current)
                            .width(ui.available_width() - 8.0)
                            .show_ui(ui, |ui| {
                                let is_all = self.source_filter.is_none();
                                if ui.selectable_label(is_all, "All").clicked() {
                                    self.source_filter = None;
                                }
                                for label in &unique_labels {
                                    let count = self
                                        .cues
                                        .iter()
                                        .filter(|c| {
                                            c.source_label.as_deref() == Some(label.as_str())
                                        })
                                        .count();
                                    let display = format!("{} ({})", label, count);
                                    let selected = self.source_filter.as_deref()
                                        == Some(label.as_str());
                                    if ui.selectable_label(selected, &display).clicked() {
                                        self.source_filter = Some(label.clone());
                                    }
                                }
                            });
                    });
                }

                ui.separator();

                egui::ScrollArea::vertical().show(ui, |ui| {
                    let mut actions: Vec<(i64, CueAction)> = Vec::new();

                    let cues_snapshot = self.cues.clone();
                    let source_filter = self.source_filter.clone();
                    for &status in CueStatus::all() {
                        let section_cues: Vec<&Cue> = cues_snapshot
                            .iter()
                            .filter(|c| c.status == status)
                            .filter(|c| {
                                if let Some(ref filter) = source_filter {
                                    c.source_label.as_deref() == Some(filter.as_str())
                                } else {
                                    true
                                }
                            })
                            .collect();

                        let header = format!("{} ({})", status.label(), section_cues.len());
                        let mut collapsing = egui::CollapsingHeader::new(header)
                            .id_salt(status.label())
                            .default_open(
                                status == CueStatus::Inbox || status == CueStatus::Review,
                            );
                        if status == CueStatus::Ready && self.expand_running_section {
                            collapsing = collapsing.open(Some(true));
                        }
                        collapsing.show(ui, |ui| {
                                if section_cues.is_empty() {
                                    ui.label(
                                        egui::RichText::new("(empty)")
                                            .italics()
                                            .color(egui::Color32::from_gray(120)),
                                    );
                                }
                                for cue in &section_cues {
                                    self.render_cue_card(ui, cue, &mut actions, status);
                                }
                            });
                    }

                    // Reset the expand flag after rendering
                    self.expand_running_section = false;

                    // Process actions after iteration
                    for (id, action) in actions {
                        match action {
                            CueAction::StartEdit(text) => {
                                self.editing_cue_id = Some(id);
                                self.editing_cue_text = text;
                            }
                            CueAction::CancelEdit => {
                                self.editing_cue_id = None;
                            }
                            CueAction::SaveEdit(new_text) => {
                                let _ = self.db.update_cue_text(id, &new_text);
                                self.editing_cue_id = None;
                            }
                            CueAction::MoveTo(new_status) => {
                                let _ = self.db.update_cue_status(id, new_status);
                                if new_status == CueStatus::Ready {
                                    self.expand_running_section = true;
                                    self.reload_cues();
                                    self.trigger_claude(id);
                                }
                            }
                            CueAction::Delete => {
                                let _ = self.db.delete_cue(id);
                            }
                            CueAction::Navigate(file_path, line, line_end) => {
                                let full_path = self.project_root.join(&file_path);
                                if self.current_file.as_ref() != Some(&full_path) {
                                    self.load_file(full_path);
                                } else {
                                    self.dismiss_central_overlays();
                                }
                                self.selection_start = Some(line);
                                self.selection_end = Some(line_end.unwrap_or(line));
                                self.scroll_to_line = Some(line);
                            }
                            CueAction::ShowDiff(cue_id) => {
                                if let Ok(Some(exec)) = self.db.get_latest_execution(cue_id) {
                                    if let Some(diff) = exec.diff {
                                        let cue = self
                                            .cues
                                            .iter()
                                            .find(|c| c.id == cue_id);
                                        let text = cue
                                            .map(|c| c.text.clone())
                                            .unwrap_or_default();
                                        let read_only = cue
                                            .map(|c| c.status != CueStatus::Review)
                                            .unwrap_or(true);
                                        let parsed = diff_view::parse_unified_diff(&diff);
                                        self.diff_review = Some(super::DiffReview {
                                            cue_id,
                                            diff,
                                            cue_text: text,
                                            parsed,
                                            view_mode: DiffViewMode::Inline,
                                            read_only,
                                            collapsed_files: HashSet::new(),
                                            prompt_expanded: false,
                                        });
                                    }
                                }
                            }
                            CueAction::CommitReview(cue_id) => {
                                if let Ok(Some(exec)) =
                                    self.db.get_latest_execution(cue_id)
                                {
                                    if let Some(ref diff) = exec.diff {
                                        let cue_text = self
                                            .cues
                                            .iter()
                                            .find(|c| c.id == cue_id)
                                            .map(|c| c.text.clone())
                                            .unwrap_or_default();
                                        let commit_msg =
                                            git::generate_commit_message(&cue_text);
                                        match git::commit_diff(
                                            &self.project_root,
                                            diff,
                                            &commit_msg,
                                        ) {
                                            Ok(hash) => {
                                                eprintln!("Committed: {}", hash);
                                                let _ = self.db.update_cue_status(
                                                    cue_id,
                                                    CueStatus::Done,
                                                );
                                            }
                                            Err(e) => {
                                                eprintln!("Commit failed: {}", e);
                                            }
                                        }
                                    }
                                }
                                self.reload_git_info();
                                self.reload_commit_history();
                            }
                            CueAction::RevertReview(cue_id) => {
                                if let Ok(Some(exec)) =
                                    self.db.get_latest_execution(cue_id)
                                {
                                    if let Some(ref diff) = exec.diff {
                                        let file_paths = git::parse_diff_file_paths_for_repo(&self.project_root,diff);
                                        if let Err(e) = git::revert_files(
                                            &self.project_root,
                                            &file_paths,
                                        ) {
                                            eprintln!("Revert failed: {}", e);
                                        }
                                    }
                                }
                                let _ = self.db.update_cue_status(
                                    cue_id,
                                    CueStatus::Inbox,
                                );
                                // Reload file to show reverted content
                                if let Some(ref path) = self.current_file {
                                    let p = path.clone();
                                    self.load_file(p);
                                }
                                self.reload_git_info();
                            }
                            CueAction::ShowRunningLog(cue_id) => {
                                // Load log from DB if not already in memory
                                if !self.running_logs.contains_key(&cue_id) {
                                    if let Ok(Some(exec)) =
                                        self.db.get_latest_execution(cue_id)
                                    {
                                        if let Some(log_text) = exec.log {
                                            self.running_logs.insert(
                                                cue_id,
                                                std::sync::Arc::new(
                                                    std::sync::Mutex::new(log_text),
                                                ),
                                            );
                                        }
                                    }
                                }
                                self.show_running_log = Some(cue_id);
                            }
                        }
                        self.reload_cues();
                    }
                });
            });
    }

    fn render_cue_card(
        &mut self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
        status: CueStatus,
    ) {
        let fs = self.settings.font_size;
        egui::Frame::none()
            .inner_margin(4.0)
            .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(60)))
            .rounding(4.0)
            .show(ui, |ui| {
                // Cue text - inline editable for Inbox
                if self.editing_cue_id == Some(cue.id) {
                    let response = ui.text_edit_multiline(&mut self.editing_cue_text);
                    ui.horizontal(|ui| {
                        if ui.small_button(icon("\u{2713} Save", fs)).clicked() {
                            actions.push((
                                cue.id,
                                CueAction::SaveEdit(self.editing_cue_text.clone()),
                            ));
                        }
                        if ui.small_button(icon("\u{2715} Cancel", fs)).clicked() {
                            actions.push((cue.id, CueAction::CancelEdit));
                        }
                    });
                    // Request focus on first frame
                    if response.gained_focus() || !response.has_focus() {
                        response.request_focus();
                    }
                } else {
                    let display_text = if cue.text.len() > 60 {
                        format!("{}...", &cue.text[..57])
                    } else {
                        cue.text.clone()
                    };
                    let label_response = ui.label(&display_text);
                    // Double-click label to edit (Inbox only)
                    if status == CueStatus::Inbox && label_response.double_clicked() {
                        actions.push((
                            cue.id,
                            CueAction::StartEdit(cue.text.clone()),
                        ));
                    }
                    // Single-click to show diff (Review/Done/Archived)
                    if matches!(status, CueStatus::Review | CueStatus::Done | CueStatus::Archived)
                        && label_response.clicked()
                    {
                        actions.push((cue.id, CueAction::ShowDiff(cue.id)));
                    }
                }

                // Source label badge
                if let Some(ref label) = cue.source_label {
                    ui.horizontal(|ui| {
                        let badge_color = source_label_color(label);
                        let badge = egui::RichText::new(label)
                            .small()
                            .background_color(badge_color)
                            .color(egui::Color32::from_gray(220));
                        ui.label(badge);
                    });
                }

                // File:line link or "Global" label
                if cue.file_path.is_empty() {
                    ui.label(
                        egui::RichText::new("Global")
                            .small()
                            .color(egui::Color32::from_rgb(180, 140, 255)),
                    );
                } else {
                    let location = if let Some(end) = cue.line_number_end {
                        format!("{}:{}-{}", cue.file_path, cue.line_number, end)
                    } else {
                        format!("{}:{}", cue.file_path, cue.line_number)
                    };
                    if ui
                        .small_button(&location)
                        .on_hover_text("Navigate to this location")
                        .clicked()
                    {
                        actions.push((
                            cue.id,
                            CueAction::Navigate(
                                cue.file_path.clone(),
                                cue.line_number,
                                cue.line_number_end,
                            ),
                        ));
                    }
                }

                // Action buttons
                ui.horizontal(|ui| {
                    match cue.status {
                        CueStatus::Inbox => {
                            if self.editing_cue_id != Some(cue.id) {
                                if ui
                                    .small_button("Edit")
                                    .on_hover_text("Edit cue")
                                    .clicked()
                                {
                                    actions.push((
                                        cue.id,
                                        CueAction::StartEdit(cue.text.clone()),
                                    ));
                                }
                            }
                            if ui
                                .small_button(icon("\u{25B6} Run", fs))
                                .on_hover_text("Send to Claude")
                                .clicked()
                            {
                                actions.push((
                                    cue.id,
                                    CueAction::MoveTo(CueStatus::Ready),
                                ));
                            }
                            if ui
                                .small_button(icon("\u{2713} Done", fs))
                                .on_hover_text("Mark done (no Claude)")
                                .clicked()
                            {
                                actions.push((
                                    cue.id,
                                    CueAction::MoveTo(CueStatus::Done),
                                ));
                            }
                        }
                        CueStatus::Ready => {
                            let elapsed = self.format_elapsed(cue.id);
                            let label = if elapsed.is_empty() {
                                "\u{2022} Running...".to_string()
                            } else {
                                format!("\u{2022} Running... {}", elapsed)
                            };
                            if ui
                                .small_button(
                                    icon(&label, fs)
                                        .color(egui::Color32::from_rgb(100, 180, 255)),
                                )
                                .on_hover_text("View Claude's progress")
                                .clicked()
                            {
                                actions.push((
                                    cue.id,
                                    CueAction::ShowRunningLog(cue.id),
                                ));
                            }
                            ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
                            if ui
                                .small_button(icon("\u{2715} Cancel", fs))
                                .on_hover_text("Cancel and move back to Inbox")
                                .clicked()
                            {
                                actions.push((
                                    cue.id,
                                    CueAction::MoveTo(CueStatus::Inbox),
                                ));
                            }
                        }
                        CueStatus::Review => {
                            if ui
                                .small_button(icon("\u{25B6} Diff", fs))
                                .on_hover_text("View the diff")
                                .clicked()
                            {
                                actions
                                    .push((cue.id, CueAction::ShowDiff(cue.id)));
                            }
                            if ui
                                .small_button(icon("Log", fs))
                                .on_hover_text("View Claude's output log")
                                .clicked()
                            {
                                actions.push((
                                    cue.id,
                                    CueAction::ShowRunningLog(cue.id),
                                ));
                            }
                            if ui
                                .small_button(icon("\u{2713} Commit", fs))
                                .on_hover_text("Commit the applied changes")
                                .clicked()
                            {
                                actions.push((
                                    cue.id,
                                    CueAction::CommitReview(cue.id),
                                ));
                            }
                            if ui
                                .small_button(icon("\u{21BA} Revert", fs))
                                .on_hover_text("Revert changes and move back to Inbox")
                                .clicked()
                            {
                                actions.push((
                                    cue.id,
                                    CueAction::RevertReview(cue.id),
                                ));
                            }
                        }
                        CueStatus::Done => {
                            ui.label(
                                icon("\u{2713}", fs)
                                    .color(egui::Color32::from_rgb(100, 200, 100)),
                            );
                            if ui
                                .small_button(icon("Log", fs))
                                .on_hover_text("View Claude's output log")
                                .clicked()
                            {
                                actions.push((
                                    cue.id,
                                    CueAction::ShowRunningLog(cue.id),
                                ));
                            }
                            if ui
                                .small_button(icon("\u{2193} Archive", fs))
                                .on_hover_text("Move to Archived")
                                .clicked()
                            {
                                actions.push((
                                    cue.id,
                                    CueAction::MoveTo(CueStatus::Archived),
                                ));
                            }
                            if ui
                                .small_button(icon("\u{21BA} Reopen", fs))
                                .on_hover_text("Move back to Inbox")
                                .clicked()
                            {
                                actions.push((
                                    cue.id,
                                    CueAction::MoveTo(CueStatus::Inbox),
                                ));
                            }
                        }
                        CueStatus::Archived => {
                            if ui
                                .small_button(icon("Log", fs))
                                .on_hover_text("View Claude's output log")
                                .clicked()
                            {
                                actions.push((
                                    cue.id,
                                    CueAction::ShowRunningLog(cue.id),
                                ));
                            }
                            if ui
                                .small_button(icon("\u{21BA} Unarchive", fs))
                                .on_hover_text("Move back to Done")
                                .clicked()
                            {
                                actions.push((
                                    cue.id,
                                    CueAction::MoveTo(CueStatus::Done),
                                ));
                            }
                        }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .small_button(icon("\u{2715}", fs))
                            .on_hover_text("Delete cue")
                            .clicked()
                        {
                            actions.push((cue.id, CueAction::Delete));
                        }
                    });
                });
            });

        ui.add_space(2.0);
    }
}

/// Pick a deterministic badge color based on the source label string.
fn source_label_color(label: &str) -> egui::Color32 {
    let hash = label
        .bytes()
        .fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));
    let colors = [
        egui::Color32::from_rgb(50, 90, 150),
        egui::Color32::from_rgb(120, 65, 120),
        egui::Color32::from_rgb(140, 85, 45),
        egui::Color32::from_rgb(45, 115, 85),
        egui::Color32::from_rgb(140, 50, 65),
        egui::Color32::from_rgb(65, 110, 140),
        egui::Color32::from_rgb(100, 100, 50),
        egui::Color32::from_rgb(80, 60, 130),
    ];
    colors[(hash as usize) % colors.len()]
}
