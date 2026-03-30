use std::path::PathBuf;

use eframe::egui;

use super::{DirigentApp, SEARCH_PANEL_DEFAULT_WIDTH, SEARCH_PANEL_MIN_WIDTH};

impl DirigentApp {
    /// Handle drag-and-drop of files onto the window.
    pub(super) fn handle_drag_and_drop(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if !dropped.is_empty() {
            if self.claude.show_log.is_some() {
                self.conversation_reply_images.extend(dropped);
            } else {
                self.global_prompt_images.extend(dropped);
            }
        }
        self.render_drop_overlay(ctx);
    }

    /// Show overlay when files are being dragged over the window.
    pub(super) fn render_drop_overlay(&self, ctx: &egui::Context) {
        if ctx.input(|i| i.raw.hovered_files.is_empty()) {
            return;
        }
        let screen = ctx.content_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("drop_overlay"),
        ));
        painter.rect_filled(screen, 0, egui::Color32::from_black_alpha(160));
        painter.text(
            screen.center(),
            egui::Align2::CENTER_CENTER,
            "Drop files to attach",
            egui::FontId::proportional(24.0),
            egui::Color32::WHITE,
        );
    }

    /// Handle global keyboard shortcuts (Cmd+N, Cmd+W, Cmd+P, Cmd+[, Cmd+]).
    pub(super) fn handle_global_shortcuts(&mut self, ctx: &egui::Context) {
        self.handle_search_shortcuts(ctx);
        if ctx.input(|i| i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::N)) {
            crate::spawn_new_instance();
        }
        if ctx.input(|i| i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::W)) {
            // Notify LSP before closing
            if self.settings.lsp_enabled {
                if let Some(tab) = self.viewer.active() {
                    let path = tab.file_path.clone();
                    self.lsp.notify_file_closed(&path);
                }
            }
            self.viewer.close_active_tab();
        }
        if ctx.input(|i| i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::P)) {
            self.viewer.quick_open_active = !self.viewer.quick_open_active;
            self.viewer.quick_open_query.clear();
            self.viewer.quick_open_selected = 0;
        }
        if ctx.input(|i| {
            i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::OpenBracket)
        }) {
            self.push_nav_history();
            self.nav_back();
        }
        if ctx.input(|i| {
            i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::CloseBracket)
        }) {
            self.nav_forward();
        }
    }

    /// Render main layout panels and all floating dialogs.
    pub(super) fn render_panels_and_dialogs(&mut self, ui: &mut egui::Ui) {
        self.render_menu_bar(ui);
        self.render_repo_bar(ui);
        self.render_status_bar(ui);
        self.render_prompt_field(ui);
        if self.search.in_files_active {
            self.render_search_in_files_panel_wrapper(ui);
        } else {
            self.render_file_tree_panel(ui);
        }
        self.render_cue_pool(ui);
        self.render_code_viewer(ui);
        let ctx = ui.ctx().clone();
        self.render_modal_overlay(&ctx);
        self.render_floating_dialogs(&ctx);
    }

    /// Render modal overlay dimming behind floating windows.
    pub(super) fn render_modal_overlay(&mut self, ctx: &egui::Context) {
        let has_modal = self.show_repo_picker
            || self.git.show_worktree_panel
            || self.show_about
            || self.pending_play.is_some()
            || self.git.show_create_pr
            || self.git.pending_force_remove.is_some()
            || self.git.pending_delete_archive.is_some()
            || self.pending_file_delete.is_some()
            || self.rename_target.is_some()
            || self.git_init_confirm.is_some()
            || self.git.show_pull_diverged
            || self.git.show_pull_unmerged
            || self.git.show_merge_conflicts
            || self.git.show_import_pr
            || self.git.show_pr_filter;
        if !has_modal {
            return;
        }
        let screen = ctx.content_rect();
        egui::Area::new(egui::Id::new("modal_dim"))
            .order(egui::Order::Middle)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let (rect, resp) = ui.allocate_exact_size(screen.size(), egui::Sense::click());
                ui.painter()
                    .rect_filled(rect, 0.0, self.semantic.modal_overlay());
                if resp.clicked() {
                    self.dismiss_topmost_modal(true);
                }
            });
    }

    /// Dismiss the topmost modal dialog (priority order).
    /// When `keep_pending_play` is true, `pending_play` is left intact
    /// (used for backdrop/overlay clicks so selections are not lost).
    pub(super) fn dismiss_topmost_modal(&mut self, keep_pending_play: bool) {
        if self.pending_play.is_some() {
            // Clear unless caller wants to keep the selection intact.
            self.pending_play = self.pending_play.take().filter(|_| keep_pending_play);
            return;
        }
        if self.git.pending_force_remove.is_some() {
            self.git.pending_force_remove = None;
        } else if self.git.pending_delete_archive.is_some() {
            self.git.pending_delete_archive = None;
        } else if self.pending_file_delete.is_some() {
            self.pending_file_delete = None;
        } else if self.rename_target.is_some() {
            self.rename_target = None;
        } else if self.git_init_confirm.is_some() {
            self.git_init_confirm = None;
        } else if self.git.show_merge_conflicts {
            self.git.show_merge_conflicts = false;
        } else if self.git.show_pull_diverged {
            self.git.show_pull_diverged = false;
        } else if self.git.show_pull_unmerged {
            self.git.show_pull_unmerged = false;
        } else if self.git.show_pr_filter {
            self.git.show_pr_filter = false;
            self.git.pr_findings_pending.clear();
            self.git.pr_findings_excluded.clear();
        } else if self.git.show_import_pr {
            self.git.show_import_pr = false;
        } else if self.git.show_create_pr {
            self.git.show_create_pr = false;
        } else if self.show_about {
            self.show_about = false;
        } else if self.git.show_worktree_panel {
            self.git.show_worktree_panel = false;
        } else if self.show_repo_picker {
            self.show_repo_picker = false;
            self.cached_existing_repos.clear();
        }
    }

    /// Render all floating dialog windows.
    pub(super) fn render_floating_dialogs(&mut self, ctx: &egui::Context) {
        self.render_repo_picker(ctx);
        self.render_worktree_panel(ctx);
        self.render_force_remove_dialog(ctx);
        self.render_delete_archive_dialog(ctx);
        self.render_file_delete_dialog(ctx);
        self.render_rename_dialog(ctx);
        self.render_about_dialog(ctx);
        self.render_play_variables_dialog(ctx);
        self.render_git_init_dialog(ctx);
        self.render_create_pr_dialog(ctx);
        self.render_pull_diverged_dialog(ctx);
        self.render_pull_unmerged_dialog(ctx);
        self.render_merge_conflicts_dialog(ctx);
        self.render_import_pr_dialog(ctx);
        self.render_filter_pr_dialog(ctx);
    }

    /// Render project-wide search panel as a left side panel (replaces file tree).
    pub(super) fn render_search_in_files_panel_wrapper(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("search_files_panel")
            .default_size(SEARCH_PANEL_DEFAULT_WIDTH)
            .min_size(SEARCH_PANEL_MIN_WIDTH)
            .max_size(400.0)
            .show_inside(ui, |ui| {
                self.render_search_in_files_panel(ui);
            });
    }
}
