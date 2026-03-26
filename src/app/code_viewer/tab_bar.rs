use eframe::egui;

use super::types::TabBarAction;
use crate::app::DirigentApp;

impl DirigentApp {
    /// Render the tab bar and return the action to take.
    pub(in crate::app) fn render_tab_bar(
        &mut self,
        ui: &mut egui::Ui,
        active_idx: usize,
    ) -> TabBarAction {
        let mut action = TabBarAction::None;

        ui.horizontal(|ui| {
            for i in 0..self.viewer.tabs.len() {
                self.render_single_tab(ui, i, i == active_idx, &mut action);
            }
        });

        ui.separator();
        action
    }

    /// Render one tab in the tab bar.
    fn render_single_tab(
        &self,
        ui: &mut egui::Ui,
        i: usize,
        is_active: bool,
        action: &mut TabBarAction,
    ) {
        let tab = &self.viewer.tabs[i];
        let filename = tab
            .file_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "untitled".to_string());

        let text = tab_label_text(
            &filename,
            is_active,
            self.semantic.accent,
            self.semantic.secondary_text,
        );

        let frame = if is_active {
            egui::Frame::NONE
                .inner_margin(egui::Margin::symmetric(6, 3))
                .fill(self.semantic.selection_bg())
                .corner_radius(3)
        } else {
            egui::Frame::NONE.inner_margin(egui::Margin::symmetric(6, 3))
        };

        let rel = tab
            .file_path
            .strip_prefix(&self.project_root)
            .unwrap_or(&tab.file_path)
            .to_string_lossy()
            .to_string();

        let tab_resp = frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui
                    .add(egui::Label::new(text).sense(egui::Sense::click()))
                    .on_hover_text(&rel)
                    .clicked()
                {
                    *action = TabBarAction::Activate(i);
                }
                if ui
                    .add(
                        egui::Label::new(
                            egui::RichText::new("\u{00D7}")
                                .small()
                                .color(self.semantic.tertiary_text),
                        )
                        .sense(egui::Sense::click()),
                    )
                    .on_hover_text("Close tab")
                    .clicked()
                {
                    *action = TabBarAction::CloseOne(i);
                }
            });
        });

        let ctx_resp = ui.interact(
            tab_resp.response.rect,
            ui.id().with(("tab_ctx", i)),
            egui::Sense::click(),
        );
        if ctx_resp.clicked() && *action != TabBarAction::CloseOne(i) {
            *action = TabBarAction::Activate(i);
        }
        self.render_tab_context_menu(&ctx_resp, i, action);
    }

    /// Show the right-click context menu on a tab.
    fn render_tab_context_menu(
        &self,
        ctx_resp: &egui::Response,
        tab_index: usize,
        action: &mut TabBarAction,
    ) {
        ctx_resp.context_menu(|ui| {
            if ui.button("Close").clicked() {
                *action = TabBarAction::CloseOne(tab_index);
                ui.close();
            }
            if ui.button("Close Others").clicked() {
                *action = TabBarAction::CloseOthers(tab_index);
                ui.close();
            }
            if ui.button("Close All").clicked() {
                *action = TabBarAction::CloseAll;
                ui.close();
            }
            if ui.button("Close Tabs to the Right").clicked() {
                *action = TabBarAction::CloseToRight(tab_index);
                ui.close();
            }
        });
    }

    /// Apply a tab bar action. Returns true if the caller should return early.
    pub(in crate::app) fn apply_tab_bar_action(&mut self, action: TabBarAction) -> bool {
        match action {
            TabBarAction::None => false,
            TabBarAction::CloseAll => {
                self.viewer.close_all_tabs();
                self.search.in_file_active = false;
                self.search.in_file_query.clear();
                self.search.in_file_matches.clear();
                self.search.in_file_current = None;
                true
            }
            TabBarAction::CloseOthers(idx) => {
                self.viewer.close_other_tabs(idx);
                self.search.in_file_active = false;
                self.search.in_file_query.clear();
                self.search.in_file_matches.clear();
                self.search.in_file_current = None;
                true
            }
            TabBarAction::CloseToRight(idx) => {
                self.viewer.close_tabs_to_right(idx);
                self.search.in_file_active = false;
                self.search.in_file_query.clear();
                self.search.in_file_matches.clear();
                self.search.in_file_current = None;
                true
            }
            TabBarAction::CloseOne(idx) => {
                self.viewer.close_tab(idx);
                self.search.in_file_active = false;
                self.search.in_file_query.clear();
                self.search.in_file_matches.clear();
                self.search.in_file_current = None;
                true
            }
            TabBarAction::Activate(idx) => {
                self.viewer.active_tab = Some(idx);
                self.search.in_file_active = false;
                self.search.in_file_query.clear();
                self.search.in_file_matches.clear();
                self.search.in_file_current = None;
                true
            }
        }
    }
}

/// Build the tab label text with proper styling.
pub(crate) fn tab_label_text(
    filename: &str,
    is_active: bool,
    accent: egui::Color32,
    secondary: egui::Color32,
) -> egui::RichText {
    if is_active {
        egui::RichText::new(filename).small().strong().color(accent)
    } else {
        egui::RichText::new(filename).small().color(secondary)
    }
}
