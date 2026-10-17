use std::path::PathBuf;

use eframe::egui;

use super::{
    DialogId, DirigentApp, SEARCH_PANEL_DEFAULT_WIDTH, SEARCH_PANEL_MIN_WIDTH, SemanticColors, icon,
};

/// A confirmation dialog whose state lives in an `Option` field rather than in
/// the [`super::DialogRegistry`]. See [`DirigentApp::pending_confirmation`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingConfirmation {
    Play,
    ForceRemoveWorktree,
    DeleteArchive,
    Trust,
    CustomTheme,
    FileDelete,
    HunkDiscard,
    Rename,
    GitInit,
    DatabaseError,
}

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
            if let Some(cue_id) = self.claude.show_log {
                self.conversation_reply_images
                    .entry(cue_id)
                    .or_default()
                    .extend(dropped);
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

    /// Whether a modal dialog is currently open. Registry dialogs are covered
    /// automatically; the `Option`-payload confirmations come from the single
    /// [`Self::pending_confirmation`] list.
    pub(super) fn has_modal_open(&self) -> bool {
        self.dialogs.any_modal() || self.pending_confirmation().is_some()
    }

    /// The topmost open `Option`-payload confirmation, in dismissal priority
    /// order (these render above registry dialogs).
    ///
    /// This is the *only* place the confirmations are enumerated: modal
    /// detection and dismissal both derive from it, so a newly added
    /// confirmation cannot land in one path and silently miss the other —
    /// the drift class that repeatedly left global shortcuts live behind an
    /// open dialog.
    fn pending_confirmation(&self) -> Option<PendingConfirmation> {
        use PendingConfirmation as P;
        if self.pending_play.is_some() {
            return Some(P::Play);
        }
        if self.git.pending_force_remove.is_some() {
            return Some(P::ForceRemoveWorktree);
        }
        if self.git.pending_delete_archive.is_some() {
            return Some(P::DeleteArchive);
        }
        if self.pending_trust.is_some() {
            return Some(P::Trust);
        }
        if self.custom_theme_edit.is_some() {
            return Some(P::CustomTheme);
        }
        if self.pending_file_delete.is_some() {
            return Some(P::FileDelete);
        }
        if self.pending_hunk_discard.is_some() {
            return Some(P::HunkDiscard);
        }
        if self.rename_target.is_some() {
            return Some(P::Rename);
        }
        if self.git_init_confirm.is_some() {
            return Some(P::GitInit);
        }
        if self.db_open_error.is_some() {
            return Some(P::DatabaseError);
        }
        None
    }

    /// Close a dialog and run its per-dialog cleanup.
    pub(super) fn close_dialog(&mut self, id: DialogId) {
        self.dialogs.close(id);
        match id {
            DialogId::PushError => self.git.push_error_message.clear(),
            DialogId::PrError => self.git.pr_error_message.clear(),
            DialogId::MoveToBranchError => self.git.move_to_branch_error_message.clear(),
            DialogId::PrFilter => {
                self.git.pr_findings_pending.clear();
                self.git.pr_findings_excluded.clear();
            }
            DialogId::Commit => self.git.reset_commit_dialog_state(),
            DialogId::RepoPicker => self.cached_existing_repos.clear(),
            _ => {}
        }
    }

    /// Handle global keyboard shortcuts (Cmd+N, Cmd+W, Cmd+P, Cmd+[, Cmd+]).
    ///
    /// Suppressed when a text edit has focus or a modal dialog is open, so
    /// they don't fire while the user is typing in the quick-open overlay,
    /// search bar, prompt field, etc.
    pub(super) fn handle_global_shortcuts(&mut self, ctx: &egui::Context) {
        self.handle_search_shortcuts(ctx);

        // Esc dismisses the topmost registry dialog — derived from the one
        // dialog list, so every dialog gets dismissal for free. Suppressed
        // while a text edit has focus (Esc there releases focus instead).
        // `Option`-payload confirmations keep their own dedicated handlers.
        if !ctx.text_edit_focused()
            && ctx.input(|i| i.key_pressed(egui::Key::Escape))
            && let Some(id) = self.dialogs.top_modal()
        {
            self.close_dialog(id);
        }

        let suppress = ctx.text_edit_focused() || self.has_modal_open();

        if command_key_pressed(ctx, suppress, egui::Key::N)
            && let Err(e) = crate::spawn_new_instance()
        {
            self.set_status_message(e);
        }
        if command_key_pressed(ctx, suppress, egui::Key::W) {
            self.notify_lsp_for_active_tab_close();
            self.viewer.close_active_tab();
        }
        if command_key_pressed(ctx, suppress, egui::Key::P) {
            self.viewer.quick_open_active = !self.viewer.quick_open_active;
            self.viewer.quick_open_query.clear();
            self.viewer.quick_open_selected = 0;
        }
        if command_key_pressed(ctx, suppress, egui::Key::OpenBracket) {
            self.push_nav_history();
            self.nav_back();
        }
        if command_key_pressed(ctx, suppress, egui::Key::CloseBracket) {
            self.nav_forward();
        }
    }

    fn notify_lsp_for_active_tab_close(&mut self) {
        if !self.settings.lsp_enabled {
            return;
        }
        if let Some(tab) = self.viewer.active() {
            let path = tab.file_path.clone();
            self.lsp.notify_file_closed(&path);
        }
    }

    /// Render main layout panels and all floating dialogs.
    pub(super) fn render_panels_and_dialogs(&mut self, ui: &mut egui::Ui) {
        self.render_menu_bar(ui);
        self.render_repo_bar(ui);
        self.render_status_bar(ui);
        self.render_prompt_field(ui);

        let t = std::time::Instant::now();
        if self.search.in_files_active {
            self.render_search_in_files_panel_wrapper(ui);
        } else if self.show_ssh_panel {
            self.render_ssh_panel(ui);
        } else if self.git.show_git_view {
            self.render_git_view_panel(ui);
        } else {
            self.render_file_tree_panel(ui);
        }
        self.last_render_file_tree_time = t.elapsed();

        let t = std::time::Instant::now();
        self.render_cue_pool(ui);
        self.last_render_cue_pool_time = t.elapsed();

        let t = std::time::Instant::now();
        self.render_code_viewer(ui);
        self.last_render_code_viewer_time = t.elapsed();

        let ctx = ui.ctx().clone();
        self.render_modal_overlay(&ctx);
        self.render_floating_dialogs(&ctx);
    }

    /// Render modal overlay dimming behind floating windows.
    pub(super) fn render_modal_overlay(&mut self, ctx: &egui::Context) {
        if !self.has_modal_open() {
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

    /// Dismiss the topmost modal dialog: `Option`-payload confirmations take
    /// priority (they render on top), then the most recently opened registry
    /// dialog. When `keep_pending_play` is true, `pending_play` is left intact
    /// (used for backdrop/overlay clicks so selections are not lost).
    pub(super) fn dismiss_topmost_modal(&mut self, keep_pending_play: bool) {
        if self.pending_play.is_some() {
            // Clear unless caller wants to keep the selection intact.
            self.pending_play = self.pending_play.take().filter(|_| keep_pending_play);
            return;
        }
        if self.dismiss_pending_confirmation() {
            return;
        }
        if let Some(id) = self.dialogs.top_modal() {
            self.close_dialog(id);
        }
    }

    /// Dismiss the topmost `Option`-payload confirmation dialog, if any.
    /// These carry their state in the `Option` itself and are not registry
    /// dialogs. Returns `true` if one was dismissed.
    ///
    /// The `match` is exhaustive, so adding a [`PendingConfirmation`] variant
    /// is a compile error until dismissal is wired up too.
    fn dismiss_pending_confirmation(&mut self) -> bool {
        use PendingConfirmation as P;
        let Some(pending) = self.pending_confirmation() else {
            return false;
        };
        match pending {
            // Handled by the caller, which decides whether to keep the
            // selection (backdrop click) or drop it (Esc).
            P::Play => return false,
            P::ForceRemoveWorktree => self.git.pending_force_remove = None,
            P::DeleteArchive => self.git.pending_delete_archive = None,
            // Dismissing the trust prompt is a decline: the repository's
            // execution-capable settings stay disabled.
            P::Trust => self.decline_pending_trust(),
            P::CustomTheme => {
                if let Some(edit) = &self.custom_theme_edit {
                    edit.ai_cancel
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                }
                self.custom_theme_edit = None;
            }
            P::FileDelete => self.pending_file_delete = None,
            P::HunkDiscard => self.pending_hunk_discard = None,
            P::Rename => self.rename_target = None,
            P::GitInit => self.git_init_confirm = None,
            // Not dismissable by backdrop/Esc: the app has no usable database
            // until the user picks one of the offered recoveries.
            P::DatabaseError => return false,
        }
        true
    }

    /// Render all floating dialog windows.
    pub(super) fn render_floating_dialogs(&mut self, ctx: &egui::Context) {
        self.render_repo_picker(ctx);
        self.render_move_to_branch_dialog(ctx);
        self.render_create_bookmark_dialog(ctx);
        self.render_delete_bookmark_dialog(ctx);
        self.render_merge_bookmark_dialog(ctx);
        self.render_switch_branch_dialog(ctx);
        self.render_worktree_panel(ctx);
        self.render_force_remove_dialog(ctx);
        self.render_delete_archive_dialog(ctx);
        self.render_file_delete_dialog(ctx);
        self.render_discard_hunk_dialog(ctx);
        self.render_rename_dialog(ctx);
        self.render_about_dialog(ctx);
        self.render_play_variables_dialog(ctx);
        self.render_git_init_dialog(ctx);
        self.render_trust_prompt_dialog(ctx);
        self.render_create_pr_dialog(ctx);
        self.render_move_to_branch_error_dialog(ctx);
        self.render_push_error_dialog(ctx);
        self.render_pr_error_dialog(ctx);
        self.render_pull_diverged_dialog(ctx);
        self.render_pull_unmerged_dialog(ctx);
        self.render_cleanup_bookmarks_dialog(ctx);
        self.render_commit_dialog(ctx);
        self.render_analyze_over(ctx);
        self.render_merge_conflicts_dialog(ctx);
        self.render_import_pr_dialog(ctx);
        self.render_filter_pr_dialog(ctx);
        self.render_custom_theme_dialog(ctx);
        self.render_log_viewer(ctx);
        self.render_mermaid_dialog(ctx);
        // Last, so it draws above everything else — until it is resolved the
        // app has no persistent storage.
        self.render_db_error_dialog(ctx);
    }

    /// Render the circular send button, vertically centered next to a text input.
    /// Returns `true` if the button was clicked.
    pub(super) fn render_send_button(
        ui: &mut egui::Ui,
        fs: f32,
        input_response: &egui::Response,
        semantic: &SemanticColors,
        tooltip: &str,
    ) -> bool {
        let mut clicked = false;
        ui.vertical_centered(|ui| {
            let input_h = input_response.rect.height();
            let btn_size = fs + 12.0;
            ui.add_space((input_h - btn_size) / 2.0);
            let send_btn = egui::Button::new(icon("\u{2191}", fs).color(semantic.accent_text()))
                .fill(semantic.accent)
                .corner_radius(btn_size as u8 / 2)
                .min_size(egui::vec2(btn_size, btn_size));
            clicked = ui.add(send_btn).on_hover_text(tooltip).clicked();
        });
        clicked
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

fn command_key_pressed(ctx: &egui::Context, suppress: bool, key: egui::Key) -> bool {
    !suppress && ctx.input(|i| i.modifiers.command && !i.modifiers.shift && i.key_pressed(key))
}
