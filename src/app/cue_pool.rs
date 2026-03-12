use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use eframe::egui;

use super::{icon, CueAction, DirigentApp, FONT_SCALE_SUBHEADING, SPACE_XS};
use crate::db::{Cue, CueStatus};
use crate::diff_view::{self, DiffViewMode};
use crate::git;
use crate::settings::CliProvider;

// -- Markdown import --

struct ImportedSection {
    number: usize,
    title: String,
    body: String,
}

fn parse_markdown_sections(content: &str) -> Vec<ImportedSection> {
    let mut sections = Vec::new();
    let mut current_title: Option<(usize, String)> = None;
    let mut body_lines: Vec<&str> = Vec::new();

    for line in content.lines() {
        if let Some(heading) = line.strip_prefix("### ") {
            // Flush previous section
            if let Some((num, title)) = current_title.take() {
                sections.push(ImportedSection {
                    number: num,
                    title,
                    body: clean_body(&body_lines),
                });
                body_lines.clear();
            }
            // Parse "N. Title" pattern
            let heading = heading.trim();
            if let Some(dot_pos) = heading.find(". ") {
                if let Ok(num) = heading[..dot_pos].parse::<usize>() {
                    current_title = Some((num, heading[dot_pos + 2..].to_string()));
                    continue;
                }
            }
            // Fallback: no number
            current_title = Some((sections.len() + 1, heading.to_string()));
        } else if current_title.is_some() {
            body_lines.push(line);
        }
    }
    // Flush last section
    if let Some((num, title)) = current_title {
        sections.push(ImportedSection {
            number: num,
            title,
            body: clean_body(&body_lines),
        });
    }

    sections
}

/// Clean up section body: strip `---` separators, code fences, and collapse
/// excessive blank lines while preserving the full content.
fn clean_body(lines: &[&str]) -> String {
    let mut out = Vec::new();
    let mut in_code_block = false;

    for &line in lines {
        let trimmed = line.trim();

        // Toggle code blocks — skip fence lines but keep code content
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }

        // Skip horizontal rules
        if !in_code_block && trimmed == "---" {
            continue;
        }

        out.push(line);
    }

    // Trim leading/trailing blank lines and collapse runs of 3+ blanks to 2
    let text = out.join("\n");
    let text = text.trim();

    let mut result = String::with_capacity(text.len());
    let mut consecutive_blanks = 0u32;
    for line in text.lines() {
        if line.trim().is_empty() {
            consecutive_blanks += 1;
            if consecutive_blanks <= 1 {
                result.push('\n');
            }
        } else {
            consecutive_blanks = 0;
            if !result.is_empty() && !result.ends_with('\n') {
                result.push('\n');
            }
            result.push_str(line);
        }
    }

    result
}

fn pick_markdown_file(start_dir: &std::path::Path) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Import Markdown Document")
        .set_directory(start_dir)
        .add_filter("Text files", &["md", "txt", "markdown"])
        .pick_file()
}

impl DirigentApp {
    pub(super) fn render_cue_pool(&mut self, ctx: &egui::Context) {
        // Clean up expired transition flashes
        self.cue_move_flash
            .retain(|_, when| when.elapsed().as_secs_f32() < 1.0);

        egui::SidePanel::right("cue_pool")
            .default_width(250.0)
            .min_width(200.0)
            .show(ctx, |ui| {
                // Header: "Cues" heading + "+" playbook button
                let mut selected_play_prompt: Option<String> = None;
                let mut custom_cue_requested = false;
                let mut import_requested = false;
                ui.horizontal(|ui| {
                    let inbox = self
                        .cues
                        .iter()
                        .filter(|c| c.status == CueStatus::Inbox)
                        .count();
                    let review = self
                        .cues
                        .iter()
                        .filter(|c| c.status == CueStatus::Review)
                        .count();
                    let counts: Vec<String> = [
                        if inbox > 0 { Some(format!("{} inbox", inbox)) } else { None },
                        if review > 0 { Some(format!("{} review", review)) } else { None },
                    ]
                    .into_iter()
                    .flatten()
                    .collect();
                    let heading_text = if counts.is_empty() {
                        "Cues".to_string()
                    } else {
                        format!("Cues ({})", counts.join(", "))
                    };
                    ui.label(egui::RichText::new(heading_text).size(self.settings.font_size * FONT_SCALE_SUBHEADING).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let plus_btn = ui.button("+").on_hover_text("Playbook");
                        if ui.button("\u{2193}").on_hover_text("Import from document").clicked() {
                            import_requested = true;
                        }
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
                    let _ = self.db.insert_cue(&prompt, "", 0, None, &[]);
                    self.reload_cues();
                }
                if custom_cue_requested {
                    // Focus the global prompt field by clearing and letting egui pick it up
                    self.global_prompt_input.clear();
                }
                if import_requested {
                    if let Some(path) = pick_markdown_file(&self.project_root) {
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            let stem = path.file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or("import")
                                .to_string();
                            let sections = parse_markdown_sections(&content);
                            let mut new_count = 0usize;
                            let mut updated_count = 0usize;
                            for section in &sections {
                                let source_ref = format!("{}#{}", path.display(), section.number);
                                let text = format!("{}\n\n{}", section.title, section.body);
                                if self.db.cue_exists_by_source_ref(&source_ref).unwrap_or(false) {
                                    if self.db.update_cue_text_by_source_ref(&source_ref, &text).is_ok() {
                                        updated_count += 1;
                                    }
                                } else {
                                    let _ = self.db.insert_cue_from_source(&text, &stem, &source_ref);
                                    new_count += 1;
                                }
                            }
                            self.reload_cues();
                            let filename = path.file_name()
                                .and_then(|s| s.to_str())
                                .unwrap_or("document");
                            let msg = match (new_count, updated_count) {
                                (0, 0) => format!("No changes from \"{}\"", filename),
                                (n, 0) => format!("Imported {} new cue(s) from \"{}\"", n, filename),
                                (0, u) => format!("Updated {} cue(s) from \"{}\"", u, filename),
                                (n, u) => format!("Imported {} new, updated {} cue(s) from \"{}\"", n, u, filename),
                            };
                            self.set_status_message(msg);
                        }
                    }
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
                        let current = self.sources.filter.as_deref().unwrap_or("All");
                        egui::ComboBox::from_id_salt("source_filter")
                            .selected_text(current)
                            .width(ui.available_width() - 8.0)
                            .show_ui(ui, |ui| {
                                let is_all = self.sources.filter.is_none();
                                if ui.selectable_label(is_all, "All").clicked() {
                                    self.sources.filter = None;
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
                                    let selected = self.sources.filter.as_deref()
                                        == Some(label.as_str());
                                    if ui.selectable_label(selected, &display).clicked() {
                                        self.sources.filter = Some(label.clone());
                                    }
                                }
                            });
                    });
                }

                ui.separator();

                egui::ScrollArea::vertical().show(ui, |ui| {
                    let mut actions: Vec<(i64, CueAction)> = Vec::new();

                    let cues_snapshot = self.cues.clone();
                    let source_filter = self.sources.filter.clone();
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

                        let header = if status == CueStatus::Archived && self.archived_cue_count > section_cues.len() {
                            format!("{} ({}/{})", status.label(), section_cues.len(), self.archived_cue_count)
                        } else {
                            format!("{} ({})", status.label(), section_cues.len())
                        };
                        let header_rt = egui::RichText::new(header)
                            .size(self.settings.font_size * FONT_SCALE_SUBHEADING)
                            .strong();
                        let mut collapsing = egui::CollapsingHeader::new(header_rt)
                            .id_salt(status.label())
                            .default_open(
                                status == CueStatus::Inbox || status == CueStatus::Review,
                            );
                        if status == CueStatus::Ready && self.claude.expand_running {
                            collapsing = collapsing.open(Some(true));
                        }
                        collapsing.show(ui, |ui| {
                                if section_cues.is_empty() {
                                    ui.label(
                                        egui::RichText::new("(empty)")
                                            .italics()
                                            .color(self.semantic.tertiary_text),
                                    );
                                }
                                // "Commit All" button for the Review column
                                if status == CueStatus::Review && section_cues.len() > 1 {
                                    if ui
                                        .small_button(icon("\u{2713} Commit All", self.settings.font_size))
                                        .on_hover_text("Commit all uncommitted changes and move all Review cues to Done")
                                        .clicked()
                                    {
                                        actions.push((0, CueAction::CommitAll));
                                    }
                                    ui.add_space(SPACE_XS);
                                }
                                for cue in &section_cues {
                                    self.render_cue_card(ui, cue, &mut actions, status);
                                }
                            });
                    }

                    // Reset the expand flag after rendering
                    self.claude.expand_running = false;

                    // Process actions after iteration
                    for (id, action) in actions {
                        match action {
                            CueAction::StartEdit(text) => {
                                self.editing_cue = Some(super::EditingCue {
                                    id,
                                    text,
                                    focus_requested: false,
                                });
                            }
                            CueAction::CancelEdit => {
                                self.editing_cue = None;
                            }
                            CueAction::SaveEdit(new_text) => {
                                let _ = self.db.update_cue_text(id, &new_text);
                                let _ = self.db.log_activity(id, "Edited");
                                self.editing_cue = None;
                            }
                            CueAction::MoveTo(new_status) => {
                                // Cancel the running task if moving away from Ready
                                if new_status != CueStatus::Ready {
                                    self.cancel_cue_task(id);
                                }
                                let _ = self.db.update_cue_status(id, new_status);
                                let _ = self.db.log_activity(id, &format!("Moved to {}", new_status.label()));
                                self.cue_move_flash.insert(id, Instant::now());
                                if new_status == CueStatus::Ready {
                                    self.claude.expand_running = true;
                                    self.reload_cues();
                                    self.trigger_claude(id);
                                }
                            }
                            CueAction::Delete => {
                                self.cancel_cue_task(id);
                                let _ = self.db.delete_cue(id);
                            }
                            CueAction::Navigate(file_path, line, line_end) => {
                                let full_path = self.project_root.join(&file_path);
                                if self.viewer.current_file.as_ref() != Some(&full_path) {
                                    self.load_file(full_path);
                                } else {
                                    self.dismiss_central_overlays();
                                }
                                self.viewer.selection_start = Some(line);
                                self.viewer.selection_end = Some(line_end.unwrap_or(line));
                                self.viewer.scroll_to_line = Some(line);
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
                                        self.dismiss_central_overlays();
                                        self.diff_review = Some(super::DiffReview {
                                            cue_id,
                                            diff,
                                            cue_text: text,
                                            parsed,
                                            view_mode: DiffViewMode::Inline,
                                            read_only,
                                            collapsed_files: HashSet::new(),
                                            prompt_expanded: false,
                                            reply_text: String::new(),
                                            search_active: false,
                                            search_query: String::new(),
                                            search_matches: Vec::new(),
                                            search_current: None,
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
                                                let short = &hash[..7.min(hash.len())];
                                                self.set_status_message(format!("Committed: {}", short));
                                                let _ = self.db.update_cue_status(
                                                    cue_id,
                                                    CueStatus::Done,
                                                );
                                                let _ = self.db.log_activity(cue_id, &format!("Committed ({})", short));
                                            }
                                            Err(e) => {
                                                self.set_status_message(format!("Commit failed: {}", e));
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
                                            self.set_status_message(format!("Revert failed: {}", e));
                                        }
                                    }
                                }
                                let _ = self.db.update_cue_status(
                                    cue_id,
                                    CueStatus::Inbox,
                                );
                                let _ = self.db.log_activity(cue_id, "Reverted");
                                // Reload file to show reverted content
                                if let Some(ref path) = self.viewer.current_file {
                                    let p = path.clone();
                                    self.load_file(p);
                                }
                                self.reload_git_info();
                            }
                            CueAction::ReplyReview(cue_id, reply_text) => {
                                self.reply_inputs.remove(&cue_id);
                                let _ = self.db.log_activity(cue_id, "Reply sent");
                                self.trigger_claude_reply(cue_id, &reply_text);
                            }
                            CueAction::ShowRunningLog(cue_id) => {
                                // Load all executions for conversation history
                                if let Ok(execs) = self.db.get_all_executions(cue_id) {
                                    // Load the latest log into running_logs if not already in memory
                                    if !self.claude.running_logs.contains_key(&cue_id) {
                                        if let Some(last) = execs.last() {
                                            if let Some(ref log_text) = last.log {
                                                self.claude.running_logs.insert(
                                                    cue_id,
                                                    (log_text.clone(), CliProvider::Claude),
                                                );
                                            }
                                        }
                                    }
                                    self.claude.conversation_history = execs;
                                }
                                self.dismiss_central_overlays();
                                self.claude.show_log = Some(cue_id);
                            }
                            CueAction::CommitAll => {
                                // Gather summaries from all Review cues
                                let review_cues: Vec<&Cue> = self
                                    .cues
                                    .iter()
                                    .filter(|c| c.status == CueStatus::Review)
                                    .collect();
                                if review_cues.is_empty() {
                                    self.set_status_message("No cues in Review".into());
                                } else {
                                    // Build commit message summarizing all Review cues
                                    let summary_lines: Vec<String> = review_cues
                                        .iter()
                                        .map(|c| {
                                            let text: String = c.text.lines().next().unwrap_or(&c.text).to_string();
                                            if text.len() > 80 {
                                                format!("- {}...", &text[..77])
                                            } else {
                                                format!("- {}", text)
                                            }
                                        })
                                        .collect();
                                    let commit_msg = format!(
                                        "Dirigent: {} cue{} committed\n\n{}",
                                        review_cues.len(),
                                        if review_cues.len() == 1 { "" } else { "s" },
                                        summary_lines.join("\n"),
                                    );
                                    let review_ids: Vec<i64> =
                                        review_cues.iter().map(|c| c.id).collect();
                                    match git::commit_all(&self.project_root, &commit_msg) {
                                        Ok(hash) => {
                                            let short = &hash[..7.min(hash.len())];
                                            self.set_status_message(format!(
                                                "Committed all: {} ({} cue{})",
                                                short,
                                                review_ids.len(),
                                                if review_ids.len() == 1 { "" } else { "s" },
                                            ));
                                            for cue_id in &review_ids {
                                                let _ = self.db.update_cue_status(
                                                    *cue_id,
                                                    CueStatus::Done,
                                                );
                                                let _ = self.db.log_activity(*cue_id, &format!("Committed ({})", short));
                                            }
                                        }
                                        Err(e) => {
                                            self.set_status_message(format!(
                                                "Commit all failed: {}",
                                                e
                                            ));
                                        }
                                    }
                                    self.reload_git_info();
                                    self.reload_commit_history();
                                }
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
        let avail_w = ui.available_width();
        let frame_resp = self.semantic.card_frame().show(ui, |ui| {
            ui.set_min_width(avail_w - 22.0);
            // Cue text - inline editable for Inbox
            let is_editing = self.editing_cue.as_ref().map(|e| e.id) == Some(cue.id);
            if is_editing {
                let editing = self.editing_cue.as_mut().unwrap();
                let response = ui.text_edit_multiline(&mut editing.text);
                ui.horizontal(|ui| {
                    if ui.small_button(icon("\u{2713} Save", fs)).clicked() {
                        actions.push((
                            cue.id,
                            CueAction::SaveEdit(self.editing_cue.as_ref().unwrap().text.clone()),
                        ));
                    }
                    if ui.small_button(icon("\u{2715} Cancel", fs)).clicked() {
                        actions.push((cue.id, CueAction::CancelEdit));
                    }
                });
                // Request focus only once when editing starts
                let editing = self.editing_cue.as_mut().unwrap();
                if !editing.focus_requested {
                    response.request_focus();
                    editing.focus_requested = true;
                }
            } else {
                let label_response = ui.add(egui::Label::new(&cue.text).wrap());
                // Double-click label to edit (Inbox/Backlog)
                if matches!(status, CueStatus::Inbox | CueStatus::Backlog)
                    && !is_editing
                    && label_response.double_clicked()
                {
                    actions.push((cue.id, CueAction::StartEdit(cue.text.clone())));
                }
                // Single-click to show diff (Review/Done/Archived)
                if matches!(
                    status,
                    CueStatus::Review | CueStatus::Done | CueStatus::Archived
                ) && label_response.clicked()
                {
                    actions.push((cue.id, CueAction::ShowDiff(cue.id)));
                }
            }

            // Source label badge and image count
            let has_badge = cue.source_label.is_some() || !cue.attached_images.is_empty();
            if has_badge {
                ui.horizontal(|ui| {
                    if let Some(ref label) = cue.source_label {
                        let badge_color = source_label_color(label);
                        let badge = egui::RichText::new(label)
                            .small()
                            .background_color(badge_color)
                            .color(self.semantic.badge_text);
                        ui.label(badge);
                    }
                    if !cue.attached_images.is_empty() {
                        ui.label(
                            egui::RichText::new(format!(
                                "{} image{}",
                                cue.attached_images.len(),
                                if cue.attached_images.len() == 1 {
                                    ""
                                } else {
                                    "s"
                                }
                            ))
                            .small()
                            .color(self.semantic.accent),
                        );
                    }
                });
            }

            // File:line link or "Global" label
            if cue.file_path.is_empty() {
                ui.label(
                    egui::RichText::new("Global")
                        .small()
                        .color(self.semantic.global_label()),
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

            // Action buttons — separated from content zone
            ui.add_space(SPACE_XS);
            ui.horizontal_wrapped(|ui| {
                match cue.status {
                    CueStatus::Inbox => {
                        if !is_editing {
                            if ui.small_button("Edit").on_hover_text("Edit cue").clicked() {
                                actions.push((cue.id, CueAction::StartEdit(cue.text.clone())));
                            }
                        }
                        if ui
                            .small_button(icon("\u{25B6} Run", fs))
                            .on_hover_text("Send to Claude")
                            .clicked()
                        {
                            actions.push((cue.id, CueAction::MoveTo(CueStatus::Ready)));
                        }
                        if ui
                            .small_button(icon("\u{2713} Done", fs))
                            .on_hover_text("Mark done (no Claude)")
                            .clicked()
                        {
                            actions.push((cue.id, CueAction::MoveTo(CueStatus::Done)));
                        }
                        if ui
                            .small_button(icon("\u{2193} Backlog", fs))
                            .on_hover_text("Move to Backlog")
                            .clicked()
                        {
                            actions.push((cue.id, CueAction::MoveTo(CueStatus::Backlog)));
                        }
                    }
                    CueStatus::Ready => {
                        let elapsed = self.format_elapsed(cue.id);
                        let label = if elapsed.is_empty() {
                            "\u{25CF} Running...".to_string()
                        } else {
                            format!("\u{25CF} Running... {}", elapsed)
                        };
                        if ui
                            .small_button(icon(&label, fs).color(self.semantic.accent))
                            .on_hover_text("View Claude's progress")
                            .clicked()
                        {
                            actions.push((cue.id, CueAction::ShowRunningLog(cue.id)));
                        }
                        ui.ctx().request_repaint_after(super::ELAPSED_REPAINT);
                        if ui
                            .small_button(icon("\u{2715} Cancel", fs))
                            .on_hover_text("Cancel and move back to Inbox")
                            .clicked()
                        {
                            actions.push((cue.id, CueAction::MoveTo(CueStatus::Inbox)));
                        }
                    }
                    CueStatus::Review => {
                        if ui
                            .small_button(icon("\u{25B6} Diff", fs))
                            .on_hover_text("View the diff")
                            .clicked()
                        {
                            actions.push((cue.id, CueAction::ShowDiff(cue.id)));
                        }
                        if ui
                            .small_button(icon("Log", fs))
                            .on_hover_text("View Claude's output log")
                            .clicked()
                        {
                            actions.push((cue.id, CueAction::ShowRunningLog(cue.id)));
                        }
                        if ui
                            .small_button(icon("\u{21A9} Reply", fs))
                            .on_hover_text("Send feedback to Claude for another iteration")
                            .clicked()
                        {
                            // Toggle reply input visibility
                            if self.reply_inputs.contains_key(&cue.id) {
                                self.reply_inputs.remove(&cue.id);
                            } else {
                                self.reply_inputs.insert(cue.id, String::new());
                            }
                        }
                        if ui
                            .small_button(icon("\u{2713} Commit", fs))
                            .on_hover_text("Commit the applied changes")
                            .clicked()
                        {
                            actions.push((cue.id, CueAction::CommitReview(cue.id)));
                        }
                        if ui
                            .small_button(icon("\u{21BA} Revert", fs))
                            .on_hover_text("Revert changes and move back to Inbox")
                            .clicked()
                        {
                            actions.push((cue.id, CueAction::RevertReview(cue.id)));
                        }
                    }
                    CueStatus::Done => {
                        ui.label(icon("\u{2713}", fs).color(self.semantic.success));
                        if ui
                            .small_button(icon("Log", fs))
                            .on_hover_text("View Claude's output log")
                            .clicked()
                        {
                            actions.push((cue.id, CueAction::ShowRunningLog(cue.id)));
                        }
                        if ui
                            .small_button(icon("\u{21A9} Reply", fs))
                            .on_hover_text("Send follow-up feedback to Claude")
                            .clicked()
                        {
                            // Toggle reply input visibility
                            if self.reply_inputs.contains_key(&cue.id) {
                                self.reply_inputs.remove(&cue.id);
                            } else {
                                self.reply_inputs.insert(cue.id, String::new());
                            }
                        }
                        if ui
                            .small_button(icon("\u{2193} Archive", fs))
                            .on_hover_text("Move to Archived")
                            .clicked()
                        {
                            actions.push((cue.id, CueAction::MoveTo(CueStatus::Archived)));
                        }
                        if ui
                            .small_button(icon("\u{21BA} Reopen", fs))
                            .on_hover_text("Move back to Inbox")
                            .clicked()
                        {
                            actions.push((cue.id, CueAction::MoveTo(CueStatus::Inbox)));
                        }
                    }
                    CueStatus::Archived => {
                        if ui
                            .small_button(icon("Log", fs))
                            .on_hover_text("View Claude's output log")
                            .clicked()
                        {
                            actions.push((cue.id, CueAction::ShowRunningLog(cue.id)));
                        }
                        if ui
                            .small_button(icon("\u{21BA} Unarchive", fs))
                            .on_hover_text("Move back to Done")
                            .clicked()
                        {
                            actions.push((cue.id, CueAction::MoveTo(CueStatus::Done)));
                        }
                    }
                    CueStatus::Backlog => {
                        if !is_editing {
                            if ui.small_button("Edit").on_hover_text("Edit cue").clicked() {
                                actions.push((cue.id, CueAction::StartEdit(cue.text.clone())));
                            }
                        }
                        if ui
                            .small_button(icon("\u{2191} Inbox", fs))
                            .on_hover_text("Move to Inbox")
                            .clicked()
                        {
                            actions.push((cue.id, CueAction::MoveTo(CueStatus::Inbox)));
                        }
                        if ui
                            .small_button(icon("\u{25B6} Run", fs))
                            .on_hover_text("Send to Claude")
                            .clicked()
                        {
                            actions.push((cue.id, CueAction::MoveTo(CueStatus::Ready)));
                        }
                    }
                }

                // Overflow menu
                let more_btn = ui.small_button("\u{2026}").on_hover_text("More actions");
                let popup_id = ui.make_persistent_id(("cue_more", cue.id));
                if more_btn.clicked() {
                    ui.memory_mut(|mem| mem.toggle_popup(popup_id));
                }
                egui::popup_below_widget(
                    ui,
                    popup_id,
                    &more_btn,
                    egui::PopupCloseBehavior::CloseOnClickOutside,
                    |ui| {
                        ui.set_min_width(140.0);
                        let is_expanded = self.logbook_expanded.contains(&cue.id);
                        let activity_label = if is_expanded {
                            "\u{25BE} Activity"
                        } else {
                            "\u{25B8} Activity"
                        };
                        if ui.button(activity_label).clicked() {
                            if is_expanded {
                                self.logbook_expanded.remove(&cue.id);
                            } else {
                                self.logbook_expanded.insert(cue.id);
                            }
                        }
                        if ui.button(icon("\u{2715} Delete", fs)).clicked() {
                            actions.push((cue.id, CueAction::Delete));
                        }
                    },
                );
            });

            // Reply input field (visible when toggled for Review/Done cues)
            if let Some(reply_text) = self.reply_inputs.get_mut(&cue.id) {
                ui.add_space(SPACE_XS);
                let response = ui.add(
                    egui::TextEdit::multiline(reply_text)
                        .desired_rows(2)
                        .desired_width(f32::INFINITY)
                        .hint_text("Describe what needs to change..."),
                );
                // Submit on Cmd+Enter
                let submit = ui
                    .small_button(icon("\u{25B6} Send", fs))
                    .on_hover_text("Send feedback to Claude (also Cmd+Enter)")
                    .clicked()
                    || (response.has_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command));
                if submit && !reply_text.trim().is_empty() {
                    actions.push((cue.id, CueAction::ReplyReview(cue.id, reply_text.clone())));
                }
            }

            // Activity logbook (shown when expanded via overflow menu)
            if self.logbook_expanded.contains(&cue.id) {
                if let Ok(entries) = self.db.get_activities(cue.id) {
                    if entries.is_empty() {
                        ui.label(
                            egui::RichText::new("No activity yet")
                                .small()
                                .color(self.semantic.muted_text()),
                        );
                    } else {
                        // Fetch agent runs for this cue (for expandable output)
                        let agent_runs = self.db.get_agent_runs_for_cue(cue.id).unwrap_or_default();

                        for entry in &entries {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(&entry.timestamp)
                                        .small()
                                        .color(self.semantic.muted_text()),
                                );
                                // Check if this is an agent event with output
                                let is_agent_event = entry.event.contains("passed")
                                    || entry.event.contains("failed")
                                    || entry.event.contains("error");
                                let agent_kind_for_event = if is_agent_event {
                                    ["Format", "Lint", "Build", "Test"]
                                        .iter()
                                        .find(|k| entry.event.starts_with(*k))
                                        .map(|k| k.to_string())
                                } else {
                                    None
                                };

                                if agent_kind_for_event.is_some() {
                                    let key = (cue.id, entry.timestamp.clone());
                                    let is_expanded = self.agent_output_expanded.contains(&key);
                                    let arrow = if is_expanded { "\u{25BE}" } else { "\u{25B8}" };
                                    if ui
                                        .add(
                                            egui::Label::new(
                                                egui::RichText::new(format!(
                                                    "{} {}",
                                                    arrow, &entry.event
                                                ))
                                                .small(),
                                            )
                                            .sense(egui::Sense::click()),
                                        )
                                        .clicked()
                                    {
                                        if is_expanded {
                                            self.agent_output_expanded.remove(&key);
                                        } else {
                                            self.agent_output_expanded.insert(key);
                                        }
                                    }
                                } else {
                                    ui.label(egui::RichText::new(&entry.event).small());
                                }
                            });

                            // Render agent output block outside the horizontal layout
                            if let Some(ref kind_label) = {
                                let is_agent_event = entry.event.contains("passed")
                                    || entry.event.contains("failed")
                                    || entry.event.contains("error");
                                if is_agent_event {
                                    ["Format", "Lint", "Build", "Test"]
                                        .iter()
                                        .find(|k| entry.event.starts_with(*k))
                                        .map(|k| k.to_string())
                                } else {
                                    None
                                }
                            } {
                                let key = (cue.id, entry.timestamp.clone());
                                if self.agent_output_expanded.contains(&key) {
                                    let kind_str = kind_label.to_lowercase();
                                    if let Some(run) = agent_runs
                                        .iter()
                                        .find(|r| {
                                            r.agent_kind == kind_str
                                                && r.started_at == entry.timestamp
                                        })
                                        .or_else(|| {
                                            agent_runs.iter().find(|r| r.agent_kind == kind_str)
                                        })
                                    {
                                        let output = if run.output.len() > 2000 {
                                            format!(
                                                "{}...\n(truncated, {} bytes total)",
                                                &run.output[..2000],
                                                run.output.len()
                                            )
                                        } else if run.output.trim().is_empty() {
                                            "(no output)".to_string()
                                        } else {
                                            run.output.clone()
                                        };
                                        egui::Frame::none()
                                            .inner_margin(egui::Margin::same(4.0))
                                            .rounding(4.0)
                                            .fill(self.semantic.selection_bg())
                                            .show(ui, |ui| {
                                                egui::ScrollArea::vertical()
                                                    .max_height(200.0)
                                                    .show(ui, |ui| {
                                                        ui.label(
                                                            egui::RichText::new(&output)
                                                                .monospace()
                                                                .small()
                                                                .color(self.semantic.muted_text()),
                                                        );
                                                    });
                                            });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });

        // Transition flash overlay when cue moves between columns
        if let Some(&when) = self.cue_move_flash.get(&cue.id) {
            let elapsed = when.elapsed().as_secs_f32();
            if elapsed < 0.6 {
                let alpha = ((0.6 - elapsed) / 0.6 * 50.0) as u8;
                let [r, g, b, _] = self.semantic.accent.to_array();
                ui.painter().rect_filled(
                    frame_resp.response.rect,
                    8.0,
                    egui::Color32::from_rgba_premultiplied(r, g, b, alpha),
                );
                ui.ctx().request_repaint();
            }
        }

        ui.add_space(SPACE_XS);
    }
}

/// Pick a deterministic badge color based on the source label string.
fn source_label_color(label: &str) -> egui::Color32 {
    let hash = label
        .bytes()
        .fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));
    let colors = [
        egui::Color32::from_rgb(60, 120, 216), // royal blue
        egui::Color32::from_rgb(163, 68, 168), // vivid purple
        egui::Color32::from_rgb(206, 120, 36), // tangerine
        egui::Color32::from_rgb(38, 154, 108), // emerald
        egui::Color32::from_rgb(210, 60, 78),  // coral
        egui::Color32::from_rgb(44, 138, 186), // cerulean
        egui::Color32::from_rgb(188, 82, 148), // magenta
        egui::Color32::from_rgb(108, 72, 190), // violet
    ];
    colors[(hash as usize) % colors.len()]
}
