use eframe::egui;

use super::super::{DialogId, DirigentApp};
use crate::diff_view::{self, DiffViewMode};

impl DirigentApp {
    /// Open the "Analyze Changes over…" picker, populating the branch list.
    pub(in crate::app) fn open_analyze_over_picker(&mut self) {
        // Don't use refresh_branch_list here: it excludes branches checked out
        // in a worktree (e.g. `main`), which are perfectly valid compare refs.
        // Every other consumer of available_branches repopulates it on open.
        self.git.available_branches =
            crate::git::list_all_branches(&self.project_root).unwrap_or_default();
        // Default the head to the current branch when known.
        if self.analyze_over_head.is_empty() {
            if let Some(info) = self.git.info.as_ref() {
                self.analyze_over_head = info.branch.clone();
            }
        }
        self.dialogs.open(DialogId::AnalyzeOverPicker);
    }

    /// Render the ref picker and the read-only results panel.
    pub(in crate::app) fn render_analyze_over(&mut self, ctx: &egui::Context) {
        self.render_analyze_over_picker(ctx);
        self.render_analyze_over_results(ctx);
    }

    fn render_analyze_over_picker(&mut self, ctx: &egui::Context) {
        if !self.dialogs.is_open(DialogId::AnalyzeOverPicker) {
            return;
        }
        let branches = self.git.available_branches.clone();
        let mut base = self.analyze_over_base.clone();
        let mut head = self.analyze_over_head.clone();
        let mut do_analyze = false;
        let mut cancel = false;

        egui::Window::new("Analyze Changes over")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.dialog_frame())
            .show(ctx, |ui| {
                render_analyze_over_picker_body(
                    ui,
                    &branches,
                    &mut base,
                    &mut head,
                    &mut do_analyze,
                    &mut cancel,
                    self.semantic.secondary_text,
                );
            });

        self.analyze_over_base = base.clone();
        self.analyze_over_head = head.clone();
        if cancel {
            self.close_dialog(DialogId::AnalyzeOverPicker);
        } else if do_analyze {
            self.start_analyze_over(base, head);
        }
    }

    fn render_analyze_over_results(&mut self, ctx: &egui::Context) {
        let Some(result) = self.analyze_over_result.as_ref() else {
            return;
        };
        let base = result.base.clone();
        let head = result.head.clone();
        let diff = result.diff.clone();
        // Snapshot the groups for read-only display (title, description, files).
        let groups: Vec<(String, String, Vec<String>)> = result
            .groups
            .iter()
            .map(|g| (g.title.clone(), g.description.clone(), g.files.clone()))
            .collect();

        let mut close = false;
        let mut view_diff = false;

        egui::Window::new(format!("Change sets: {base}…{head}"))
            .collapsible(true)
            .resizable(true)
            .default_width(480.0)
            .frame(self.semantic.dialog_frame())
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "{} change set(s) — read-only review",
                        groups.len()
                    ))
                    .small()
                    .color(self.semantic.secondary_text),
                );
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .show(ui, |ui| {
                        for (i, (title, desc, files)) in groups.iter().enumerate() {
                            render_analyze_over_group(
                                ui,
                                i + 1,
                                title,
                                desc,
                                files,
                                self.semantic.secondary_text,
                                self.semantic.tertiary_text,
                            );
                        }
                    });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("View full diff").clicked() {
                        view_diff = true;
                    }
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                });
            });

        if view_diff {
            self.open_readonly_diff(&diff, format!("{base}…{head}"), DiffViewMode::Inline);
        }
        if close {
            self.analyze_over_result = None;
        }
    }

    /// Open an arbitrary diff string in the read-only diff review overlay.
    pub(in crate::app) fn open_readonly_diff(
        &mut self,
        diff: &str,
        label: String,
        view_mode: DiffViewMode,
    ) {
        let parsed = diff_view::parse_unified_diff(diff);
        self.dismiss_central_overlays();
        self.diff_review = Some(super::super::DiffReview {
            cue_id: 0,
            diff: diff.to_string(),
            cue_text: label,
            commit_hash: None,
            commit_author: None,
            parsed,
            view_mode,
            read_only: true,
            collapsed_files: std::collections::HashSet::new(),
            prompt_expanded: false,
            reply_text: String::new(),
            search_active: false,
            search_query: String::new(),
            search_matches: Vec::new(),
            search_current: None,
            staging: None,
        });
    }
}

fn render_analyze_over_picker_body(
    ui: &mut egui::Ui,
    branches: &[String],
    base: &mut String,
    head: &mut String,
    do_analyze: &mut bool,
    cancel: &mut bool,
    secondary_text: egui::Color32,
) {
    ui.label(
        egui::RichText::new("Group the changes a branch introduced (base…head).")
            .small()
            .color(secondary_text),
    );
    ui.add_space(6.0);
    render_ref_combo(ui, "Base", branches, base);
    render_ref_combo(ui, "Head", branches, head);
    ui.add_space(8.0);
    render_analyze_over_buttons(ui, base, head, do_analyze, cancel);
}

fn render_ref_combo(ui: &mut egui::Ui, label: &str, branches: &[String], value: &mut String) {
    egui::ComboBox::from_label(label)
        .selected_text(if value.is_empty() {
            "—"
        } else {
            value.as_str()
        })
        .show_ui(ui, |ui| {
            for branch in branches {
                ui.selectable_value(value, branch.clone(), branch);
            }
        });
}

fn render_analyze_over_buttons(
    ui: &mut egui::Ui,
    base: &str,
    head: &str,
    do_analyze: &mut bool,
    cancel: &mut bool,
) {
    let eligible = !base.is_empty() && !head.is_empty() && base != head;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(eligible, egui::Button::new("Analyze"))
            .on_hover_text(if eligible {
                "Group the base…head diff"
            } else {
                "Pick two different refs"
            })
            .clicked()
        {
            *do_analyze = true;
        }
        if ui.button("Cancel").clicked() {
            *cancel = true;
        }
    });
}

fn render_analyze_over_group(
    ui: &mut egui::Ui,
    index: usize,
    title: &str,
    desc: &str,
    files: &[String],
    secondary_text: egui::Color32,
    tertiary_text: egui::Color32,
) {
    ui.group(|ui| {
        ui.label(egui::RichText::new(format!("{}. {title}", index)).strong());
        if !desc.is_empty() {
            ui.label(egui::RichText::new(desc).small().color(secondary_text));
        }
        ui.label(
            egui::RichText::new(files.join(", "))
                .small()
                .monospace()
                .color(tertiary_text),
        );
    });
}
