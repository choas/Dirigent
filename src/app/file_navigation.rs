use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::DirigentApp;
use crate::db::Cue;

impl DirigentApp {
    pub(super) fn load_file(&mut self, path: PathBuf) {
        self.dismiss_central_overlays();
        self.viewer.open_file_without_history(path);
        // Reset in-file search state when switching or opening a file
        self.search.in_file_active = false;
        self.search.in_file_query.clear();
        self.search.in_file_matches.clear();
        self.search.in_file_current = None;
    }

    /// Push the current position onto the navigation history.
    pub(super) fn push_nav_history(&mut self) {
        if let Some(tab) = self.viewer.active() {
            let line = tab.selection_start.unwrap_or(1);
            self.viewer.nav_history.push(tab.file_path.clone(), line);
        }
    }

    /// Navigate back in history.
    pub(super) fn nav_back(&mut self) {
        if let Some((path, line)) = self.viewer.nav_history.go_back() {
            self.viewer.open_file_without_history(path);
            self.viewer.scroll_to_line = Some(line);
            self.dismiss_central_overlays();
        }
    }

    /// Navigate forward in history.
    pub(super) fn nav_forward(&mut self) {
        if let Some((path, line)) = self.viewer.nav_history.go_forward() {
            self.viewer.open_file_without_history(path);
            self.viewer.scroll_to_line = Some(line);
            self.dismiss_central_overlays();
        }
    }

    pub(super) fn relative_path(&self, path: &Path) -> String {
        path.strip_prefix(&self.project_root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string()
    }

    pub(super) fn file_cues(&self) -> Vec<&Cue> {
        if let Some(current) = self.viewer.current_file() {
            let rel = self.relative_path(current);
            self.cues.iter().filter(|c| c.file_path == rel).collect()
        } else {
            Vec::new()
        }
    }

    /// Returns a map from line number to whether the cue is archived.
    /// `false` = active (yellow dot), `true` = archived (grey dot).
    /// If a line has both active and archived cues, active wins.
    pub(super) fn lines_with_cues(&self) -> HashMap<usize, bool> {
        let mut map = HashMap::new();
        for c in self.file_cues() {
            let start = c.line_number;
            let end = c.line_number_end.unwrap_or(start);
            let is_archived = c.status == crate::db::CueStatus::Archived;
            for line in start..=end {
                let entry = map.entry(line).or_insert(is_archived);
                // Active cue wins over archived on the same line
                if !is_archived {
                    *entry = false;
                }
            }
        }
        map
    }
}
