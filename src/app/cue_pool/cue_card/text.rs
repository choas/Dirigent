use std::sync::OnceLock;

use eframe::egui;
use regex::Regex;

use super::super::super::{icon, CueAction, DirigentApp};
use super::utils::compute_display_text;
use crate::db::{Cue, CueStatus};

// ---------------------------------------------------------------------------
// Inline file-reference parsing
// ---------------------------------------------------------------------------

/// A file reference found in cue text, e.g. `src/main.rs:42`.
struct InlineFileRef {
    /// Character offset range in the display text (for galley hit-testing).
    char_range: std::ops::Range<usize>,
    /// Byte offset range in the display text (for string slicing).
    byte_range: std::ops::Range<usize>,
    /// Relative file path.
    file_path: String,
    /// 1-based line number.
    line: usize,
}

fn file_ref_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // Optional "Dirigent:" prefix, optional directories, file extension, `:line`.
        Regex::new(r"(?:Dirigent:)?((?:[\w.\-]+/)*[\w.\-]+\.\w+):(\d+)")
            .expect("hardcoded file-reference regex")
    })
}

fn find_file_references(text: &str) -> Vec<InlineFileRef> {
    let re = file_ref_regex();
    re.captures_iter(text)
        .map(|cap| {
            let path_match = cap.get(1).expect("capture group 1 always present in match");
            let line_match = cap.get(2).expect("capture group 2 always present in match");
            let byte_start = path_match.start();
            let byte_end = line_match.end();
            // Convert byte offsets → character offsets for galley CCursor matching.
            let char_start = text[..byte_start].chars().count();
            let char_end = text[..byte_end].chars().count();
            InlineFileRef {
                char_range: char_start..char_end,
                byte_range: byte_start..byte_end,
                file_path: path_match.as_str().to_string(),
                line: line_match.as_str().parse().unwrap_or(1),
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

impl DirigentApp {
    pub(in crate::app) fn render_cue_text(
        &mut self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
        status: CueStatus,
    ) {
        let fs = self.settings.font_size;
        let is_editing = self.editing_cue.as_ref().map(|e| e.id) == Some(cue.id);
        if is_editing {
            self.render_editing_cue(ui, cue, actions, fs);
        } else {
            self.render_display_cue(ui, cue, actions, status);
        }
    }

    pub(in crate::app) fn render_editing_cue(
        &mut self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
        fs: f32,
    ) {
        let Some(editing) = self.editing_cue.as_mut() else {
            return;
        };
        let response = ui.text_edit_multiline(&mut editing.text);
        ui.horizontal(|ui| {
            if ui.small_button(icon("\u{2713} Save", fs)).clicked() {
                if let Some(ref editing) = self.editing_cue {
                    actions.push((cue.id, CueAction::SaveEdit(editing.text.clone())));
                }
            }
            if ui.small_button(icon("\u{2715} Cancel", fs)).clicked() {
                actions.push((cue.id, CueAction::CancelEdit));
            }
        });
        let Some(editing) = self.editing_cue.as_mut() else {
            return;
        };
        if !editing.focus_requested {
            response.request_focus();
            editing.focus_requested = true;
        }
    }

    pub(in crate::app) fn render_display_cue(
        &mut self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
        status: CueStatus,
    ) {
        let display_text = compute_display_text(cue, self.cue_text_expanded.contains(&cue.id));
        let file_refs = find_file_references(&display_text);

        if file_refs.is_empty() {
            // No file refs — plain label (existing behaviour).
            let resp = ui.add(egui::Label::new(&display_text).wrap());
            Self::handle_label_clicks(resp, cue, actions, status, false);
        } else {
            // Render with inline clickable file references.
            let (resp, nav) = self.render_text_with_refs(ui, &display_text, &file_refs);
            let navigated = if resp.clicked() { nav } else { None };
            if let Some((path, line)) = &navigated {
                actions.push((cue.id, CueAction::Navigate(path.clone(), *line, None)));
            }
            Self::handle_label_clicks(resp, cue, actions, status, navigated.is_some());
        }

        self.render_expand_collapse_toggle(ui, cue);
    }

    /// Double-click → edit (Inbox/Backlog) · click → show diff (Review/Done/Archived).
    fn handle_label_clicks(
        resp: egui::Response,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
        status: CueStatus,
        suppress: bool,
    ) {
        if suppress {
            return;
        }
        if matches!(status, CueStatus::Inbox | CueStatus::Backlog) && resp.double_clicked() {
            actions.push((cue.id, CueAction::StartEdit(cue.text.clone())));
        }
        if matches!(
            status,
            CueStatus::Review | CueStatus::Done | CueStatus::Archived
        ) && resp.clicked()
        {
            actions.push((cue.id, CueAction::ShowDiff(cue.id)));
        }
    }

    /// Build a `LayoutJob` that underlines file references, paint the galley,
    /// and return the navigation target if a file ref is hovered.
    fn render_text_with_refs(
        &self,
        ui: &mut egui::Ui,
        text: &str,
        refs: &[InlineFileRef],
    ) -> (egui::Response, Option<(String, usize)>) {
        let text_color = ui.visuals().text_color();
        let link_color = self.semantic.accent;
        let font_id = egui::TextStyle::Body.resolve(ui.style());

        // ---- build LayoutJob ----
        let mut job = egui::text::LayoutJob::default();
        let mut cursor = 0usize;

        let plain = egui::TextFormat {
            font_id: font_id.clone(),
            color: text_color,
            ..Default::default()
        };
        let link = egui::TextFormat {
            font_id: font_id.clone(),
            color: link_color,
            underline: egui::Stroke::new(1.0, link_color),
            ..Default::default()
        };

        for r in refs {
            if r.byte_range.start > cursor {
                job.append(&text[cursor..r.byte_range.start], 0.0, plain.clone());
            }
            job.append(&text[r.byte_range.clone()], 0.0, link.clone());
            cursor = r.byte_range.end;
        }
        if cursor < text.len() {
            job.append(&text[cursor..], 0.0, plain);
        }

        job.wrap.max_width = ui.available_width();

        // ---- layout & paint ----
        let galley = ui.painter().layout_job(job);
        let (rect, response) = ui.allocate_exact_size(galley.size(), egui::Sense::click());

        if ui.is_rect_visible(rect) {
            ui.painter().galley(rect.min, galley.clone(), text_color);
        }

        // ---- hit-test for hovering file refs (cursor icon) ----
        let hovering_ref = response
            .hover_pos()
            .map(|pos| {
                let cc = galley.cursor_from_pos(pos - rect.min);
                refs.iter().any(|r| r.char_range.contains(&cc.index))
            })
            .unwrap_or(false);
        if hovering_ref {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        // ---- detect click on a file ref ----
        let clicked_nav = response
            .interact_pointer_pos()
            .filter(|_| response.clicked())
            .and_then(|pos| {
                let cc = galley.cursor_from_pos(pos - rect.min);
                refs.iter()
                    .find(|r| r.char_range.contains(&cc.index))
                    .map(|r| (r.file_path.clone(), r.line))
            });

        (response, clicked_nav)
    }

    pub(in crate::app) fn render_expand_collapse_toggle(&mut self, ui: &mut egui::Ui, cue: &Cue) {
        let line_count = cue.text.lines().count();
        let word_count = cue.text.split_whitespace().count();
        let is_long = line_count > 10 || word_count > 50;
        if !is_long {
            return;
        }
        let is_expanded = self.cue_text_expanded.contains(&cue.id);
        let toggle_label = if is_expanded {
            "\u{25B4} Show less"
        } else {
            "\u{25BE} Show more"
        };
        let clicked = ui
            .add(
                egui::Label::new(
                    egui::RichText::new(toggle_label)
                        .small()
                        .color(self.semantic.accent),
                )
                .sense(egui::Sense::click()),
            )
            .clicked();
        if clicked {
            if is_expanded {
                self.cue_text_expanded.remove(&cue.id);
            } else {
                self.cue_text_expanded.insert(cue.id);
            }
        }
    }
}
