use eframe::egui;

use super::line_rendering::path_matches_diagnostic;
use super::types::BreadcrumbAction;
use crate::app::{icon, symbols, DirigentApp, FONT_SCALE_SMALL, FONT_SCALE_SUBHEADING};

impl DirigentApp {
    /// Render the breadcrumb navigation bar and the right-side controls.
    pub(in crate::app) fn render_breadcrumb_bar(
        &mut self,
        ui: &mut egui::Ui,
        active_idx: usize,
        rel_path: &str,
        is_dirty: bool,
        is_markdown: bool,
    ) -> BreadcrumbAction {
        let mut bc_action = BreadcrumbAction::None;

        ui.horizontal(|ui| {
            self.render_breadcrumb_segments(ui, active_idx, rel_path);

            if ui
                .small_button(icon("\u{1F4CB}", self.settings.font_size))
                .on_hover_text("Copy file path")
                .clicked()
            {
                ui.ctx().copy_text(rel_path.to_string());
            }
            if is_dirty {
                ui.label(egui::RichText::new("\u{25CF}").color(self.semantic.warning));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                bc_action = self.render_breadcrumb_right_controls(
                    ui,
                    active_idx,
                    rel_path,
                    is_dirty,
                    is_markdown,
                );
            });
        });

        bc_action
    }

    /// Render the right-aligned controls in the breadcrumb bar (close, diagnostics, markdown toggle, diff).
    fn render_breadcrumb_right_controls(
        &mut self,
        ui: &mut egui::Ui,
        active_idx: usize,
        rel_path: &str,
        is_dirty: bool,
        is_markdown: bool,
    ) -> BreadcrumbAction {
        if ui
            .small_button(icon("\u{2715}", self.settings.font_size))
            .on_hover_text("Close tab (\u{2318}W)")
            .clicked()
        {
            return BreadcrumbAction::CloseFile;
        }
        self.render_breadcrumb_diagnostics(ui, rel_path);
        ui.label(format!(
            "{} lines",
            self.viewer.tabs[active_idx].content.len()
        ));
        if is_markdown {
            let label = if self.viewer.tabs[active_idx].markdown_rendered {
                "Raw"
            } else {
                "Rendered"
            };
            if ui
                .small_button(label)
                .on_hover_text("Toggle between raw Markdown and rendered view")
                .clicked()
            {
                return BreadcrumbAction::ToggleMarkdown;
            }
        }
        if is_dirty
            && ui
                .small_button("Show Diff")
                .on_hover_text("Show uncommitted changes for this file")
                .clicked()
        {
            return BreadcrumbAction::ShowFileDiff;
        }
        BreadcrumbAction::None
    }

    /// Render path segments and the current symbol in the breadcrumb bar.
    fn render_breadcrumb_segments(&mut self, ui: &mut egui::Ui, active_idx: usize, rel_path: &str) {
        let segments: Vec<&str> = rel_path.split('/').collect();
        for (i, segment) in segments.iter().enumerate() {
            if i > 0 {
                ui.label(egui::RichText::new("\u{203A}").color(self.semantic.tertiary_text));
            }
            self.render_breadcrumb_segment(ui, segment, i, &segments);
        }

        let current_line = self.estimate_current_line(active_idx);
        self.render_breadcrumb_symbol(ui, active_idx, current_line);
    }

    /// Render a single breadcrumb path segment with click behavior.
    fn render_breadcrumb_segment(
        &mut self,
        ui: &mut egui::Ui,
        segment: &str,
        i: usize,
        segments: &[&str],
    ) {
        let is_last = i == segments.len() - 1;
        let text = if is_last {
            egui::RichText::new(segment)
                .size(self.settings.font_size * FONT_SCALE_SUBHEADING)
                .strong()
        } else {
            egui::RichText::new(segment)
                .size(self.settings.font_size * FONT_SCALE_SMALL)
                .color(self.semantic.secondary_text)
        };
        let hover = if is_last {
            "Scroll to top".to_string()
        } else {
            format!("Expand directory {}", segments[..=i].join("/"))
        };
        if ui
            .add(egui::Label::new(text).sense(egui::Sense::click()))
            .on_hover_text(hover)
            .clicked()
        {
            self.handle_breadcrumb_segment_click(is_last, &segments[..=i]);
        }
    }

    /// Estimate the currently visible line from selection or scroll offset.
    fn estimate_current_line(&self, active_idx: usize) -> usize {
        if let Some(line) = self.viewer.tabs[active_idx].selection_start {
            return line;
        }
        let offset = self.viewer.tabs[active_idx].scroll_offset;
        if offset > 0.0 {
            (offset / 16.0) as usize + 1
        } else {
            1
        }
    }

    /// Render the current symbol indicator in the breadcrumb bar.
    fn render_breadcrumb_symbol(&self, ui: &mut egui::Ui, active_idx: usize, current_line: usize) {
        if let Some(sym) =
            symbols::enclosing_symbol(&self.viewer.tabs[active_idx].symbols, current_line)
        {
            ui.label(egui::RichText::new("\u{203A}").color(self.semantic.tertiary_text));
            ui.label(
                egui::RichText::new(format!("{} {}", sym.kind.label(), sym.name))
                    .small()
                    .color(self.semantic.accent),
            );
        }
    }

    fn handle_breadcrumb_segment_click(&mut self, is_last: bool, path_segments: &[&str]) {
        if is_last {
            self.viewer.scroll_to_line = Some(1);
        } else {
            let dir_path = self.project_root.join(path_segments.join("/"));
            self.expanded_dirs.insert(dir_path);
        }
    }

    /// Render the diagnostic error/warning counts in the breadcrumb right side.
    fn render_breadcrumb_diagnostics(&self, ui: &mut egui::Ui, rel_path: &str) {
        let mut err_count = 0usize;
        let mut warn_count = 0usize;
        for diags in self.agent_state.latest_diagnostics.values() {
            for d in diags {
                if path_matches_diagnostic(rel_path, &d.file) {
                    match d.severity {
                        crate::agents::Severity::Error => err_count += 1,
                        crate::agents::Severity::Warning => warn_count += 1,
                        _ => {}
                    }
                }
            }
        }
        if err_count > 0 {
            ui.label(
                egui::RichText::new(format!("{} errors", err_count))
                    .small()
                    .color(self.semantic.danger),
            );
        }
        if warn_count > 0 {
            ui.label(
                egui::RichText::new(format!("{} warnings", warn_count))
                    .small()
                    .color(self.semantic.warning),
            );
        }
    }

    /// Apply a breadcrumb action. Returns true if the caller should return early.
    pub(in crate::app) fn apply_breadcrumb_action(
        &mut self,
        action: BreadcrumbAction,
        active_idx: usize,
        rel_path: &str,
    ) -> bool {
        match action {
            BreadcrumbAction::ToggleMarkdown => {
                self.viewer.tabs[active_idx].markdown_rendered =
                    !self.viewer.tabs[active_idx].markdown_rendered;
                false
            }
            BreadcrumbAction::CloseFile => {
                self.viewer.close_active_tab();
                self.search.in_file_active = false;
                self.search.in_file_query.clear();
                self.search.in_file_matches.clear();
                self.search.in_file_current = None;
                true
            }
            BreadcrumbAction::ShowFileDiff => {
                self.open_file_diff(rel_path);
                true
            }
            BreadcrumbAction::None => false,
        }
    }
}
