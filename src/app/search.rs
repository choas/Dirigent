use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use eframe::egui;

use super::{icon, DirigentApp};
use crate::file_tree::FileEntry;

/// A single match from project-wide search.
#[derive(Clone)]
pub(super) struct SearchResult {
    pub file_path: PathBuf,
    pub rel_path: String,
    pub line_number: usize,
    pub line_content: String,
}

/// Recursively collect all file paths from the file tree.
pub(super) fn collect_files(entries: &[FileEntry], out: &mut Vec<PathBuf>) {
    for entry in entries {
        if entry.is_dir {
            collect_files(&entry.children, out);
        } else {
            out.push(entry.path.clone());
        }
    }
}

const BINARY_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "bmp", "ico", "svg",
    "woff", "woff2", "ttf", "otf",
    "zip", "tar", "gz", "bz2",
    "exe", "dll", "so", "dylib",
    "o", "a", "lib",
    "pdf", "db", "sqlite",
];

fn is_binary_ext(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    BINARY_EXTENSIONS.iter().any(|&b| b == ext)
}

/// Search files in parallel across multiple threads, returning up to `max_results` matches.
pub(super) fn search_files_parallel(
    root: &Path,
    files: &[PathBuf],
    query: &str,
    max_results: usize,
) -> Vec<SearchResult> {
    if files.is_empty() {
        return Vec::new();
    }

    let query_lower = query.to_lowercase();
    let num_threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(8);
    let chunk_size = (files.len() + num_threads - 1) / num_threads;
    let found_count = AtomicUsize::new(0);

    std::thread::scope(|s| {
        let handles: Vec<_> = files
            .chunks(chunk_size)
            .map(|chunk| {
                let query_lower = &query_lower;
                let found_count = &found_count;
                let root = root;
                s.spawn(move || {
                    let mut local_results = Vec::new();
                    for file_path in chunk {
                        if found_count.load(Ordering::Relaxed) >= max_results {
                            break;
                        }
                        if is_binary_ext(file_path) {
                            continue;
                        }
                        let content = match std::fs::read_to_string(file_path) {
                            Ok(c) => c,
                            Err(_) => continue,
                        };
                        let rel_path = file_path
                            .strip_prefix(root)
                            .unwrap_or(file_path)
                            .to_string_lossy()
                            .to_string();
                        for (idx, line) in content.lines().enumerate() {
                            if line.to_lowercase().contains(query_lower.as_str()) {
                                local_results.push(SearchResult {
                                    file_path: file_path.clone(),
                                    rel_path: rel_path.clone(),
                                    line_number: idx + 1,
                                    line_content: line.to_string(),
                                });
                                found_count.fetch_add(1, Ordering::Relaxed);
                                if found_count.load(Ordering::Relaxed) >= max_results {
                                    break;
                                }
                            }
                        }
                    }
                    local_results
                })
            })
            .collect();

        let mut all_results: Vec<SearchResult> = handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect();
        all_results.truncate(max_results);
        all_results
    })
}

impl DirigentApp {
    /// Update search-in-file matches when the query changes.
    pub(super) fn update_search_in_file_matches(&mut self) {
        self.search.in_file_matches.clear();
        self.search.in_file_current = None;
        if self.search.in_file_query.is_empty() {
            return;
        }
        let query = self.search.in_file_query.to_lowercase();
        for (idx, line) in self.viewer.content.iter().enumerate() {
            if line.to_lowercase().contains(&query) {
                self.search.in_file_matches.push(idx + 1);
            }
        }
        if !self.search.in_file_matches.is_empty() {
            self.search.in_file_current = Some(0);
            self.viewer.scroll_to_line = Some(self.search.in_file_matches[0]);
        }
    }

    /// Navigate to the next match in the current file.
    pub(super) fn search_in_file_next(&mut self) {
        if self.search.in_file_matches.is_empty() {
            return;
        }
        let next = match self.search.in_file_current {
            Some(i) => (i + 1) % self.search.in_file_matches.len(),
            None => 0,
        };
        self.search.in_file_current = Some(next);
        self.viewer.scroll_to_line = Some(self.search.in_file_matches[next]);
    }

    /// Navigate to the previous match in the current file.
    pub(super) fn search_in_file_prev(&mut self) {
        if self.search.in_file_matches.is_empty() {
            return;
        }
        let prev = match self.search.in_file_current {
            Some(0) => self.search.in_file_matches.len() - 1,
            Some(i) => i - 1,
            None => 0,
        };
        self.search.in_file_current = Some(prev);
        self.viewer.scroll_to_line = Some(self.search.in_file_matches[prev]);
    }

    /// Render the search-in-file bar (shown at top of code viewer when active).
    /// Returns true if the bar was closed.
    pub(super) fn render_search_in_file_bar(&mut self, ui: &mut egui::Ui) -> bool {
        let mut close = false;
        let fs = self.settings.font_size;

        ui.horizontal(|ui| {
            ui.label("Find:");
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.search.in_file_query)
                    .desired_width(250.0)
                    .hint_text("Search in file...")
                    .font(egui::TextStyle::Monospace),
            );

            response.request_focus();

            if response.changed() {
                self.update_search_in_file_matches();
            }

            // Enter = next, Shift+Enter = prev
            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                if ui.input(|i| i.modifiers.shift) {
                    self.search_in_file_prev();
                } else {
                    self.search_in_file_next();
                }
                response.request_focus();
            }

            let match_count = self.search.in_file_matches.len();
            if !self.search.in_file_query.is_empty() {
                let label = if match_count == 0 {
                    "No matches".to_string()
                } else {
                    let current = self.search.in_file_current.map(|i| i + 1).unwrap_or(0);
                    format!("{}/{}", current, match_count)
                };
                ui.label(
                    egui::RichText::new(label)
                        .monospace()
                        .small()
                        .color(if match_count == 0 {
                            egui::Color32::from_rgb(220, 100, 100)
                        } else {
                            egui::Color32::from_gray(160)
                        }),
                );
            }

            if ui.small_button(icon("\u{2191}", fs)).on_hover_text("Previous (Shift+Enter)").clicked() {
                self.search_in_file_prev();
            }
            if ui.small_button(icon("\u{2193}", fs)).on_hover_text("Next (Enter)").clicked() {
                self.search_in_file_next();
            }
            if ui.small_button(icon("\u{2715}", fs)).on_hover_text("Close (Esc)").clicked() {
                close = true;
            }

            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                close = true;
            }
        });
        ui.separator();

        if close {
            self.search.in_file_active = false;
            self.search.in_file_query.clear();
            self.search.in_file_matches.clear();
            self.search.in_file_current = None;
        }

        close
    }

    /// Render the project-wide search panel (replaces file tree when active).
    pub(super) fn render_search_in_files_panel(&mut self, ui: &mut egui::Ui) {
        let fs = self.settings.font_size;

        ui.horizontal(|ui| {
            ui.strong("Search in Files");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button(icon("\u{2715}", fs)).on_hover_text("Close search").clicked() {
                    self.search.in_files_active = false;
                }
            });
        });
        ui.separator();

        let mut trigger_search = false;
        ui.horizontal(|ui| {
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.search.in_files_query)
                    .desired_width(ui.available_width() - 40.0)
                    .hint_text("Search...")
                    .font(egui::TextStyle::Monospace),
            );
            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                trigger_search = true;
            }
            if ui.small_button(icon("\u{1F50D}", fs)).clicked() {
                trigger_search = true;
            }
        });

        if trigger_search && !self.search.in_files_query.is_empty() {
            if let Some(ref tree) = self.file_tree {
                self.search.in_files_searching = true;
                self.search.in_files_results.clear();
                let root = self.project_root.clone();
                let query = self.search.in_files_query.clone();
                let mut files = Vec::new();
                collect_files(&tree.entries, &mut files);
                let tx = self.search.search_result_tx.clone();
                std::thread::spawn(move || {
                    let results = search_files_parallel(&root, &files, &query, 500);
                    let _ = tx.send(results);
                });
            }
        }

        ui.separator();

        if !self.search.in_files_results.is_empty() {
            ui.label(
                egui::RichText::new(format!(
                    "{} results{}",
                    self.search.in_files_results.len(),
                    if self.search.in_files_results.len() >= 500 {
                        " (capped)"
                    } else {
                        ""
                    }
                ))
                .small()
                .color(egui::Color32::from_gray(140)),
            );
            ui.separator();
        }

        let mut navigate_to: Option<(PathBuf, usize)> = None;

        egui::ScrollArea::vertical()
            .id_salt("search_files_scroll")
            .show(ui, |ui| {
                if self.search.in_files_searching {
                    ui.label(
                        egui::RichText::new("Searching...")
                            .italics()
                            .color(egui::Color32::from_gray(120)),
                    );
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
                } else if self.search.in_files_results.is_empty() && !self.search.in_files_query.is_empty()
                {
                    ui.label(
                        egui::RichText::new("No results found.")
                            .italics()
                            .color(egui::Color32::from_gray(120)),
                    );
                }

                let mut current_file: Option<&str> = None;
                for result in &self.search.in_files_results {
                    if current_file != Some(&result.rel_path) {
                        current_file = Some(&result.rel_path);
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new(&result.rel_path)
                                .strong()
                                .small()
                                .color(egui::Color32::from_rgb(100, 180, 255)),
                        );
                    }

                    let line_label = format!("{:>4}:", result.line_number);
                    let content_preview = if result.line_content.len() > 80 {
                        format!("{}...", &result.line_content[..77])
                    } else {
                        result.line_content.clone()
                    };
                    let text = format!("{} {}", line_label, content_preview.trim());

                    if ui
                        .selectable_label(
                            false,
                            egui::RichText::new(&text).monospace().small(),
                        )
                        .clicked()
                    {
                        navigate_to = Some((result.file_path.clone(), result.line_number));
                    }
                }
            });

        if let Some((path, line)) = navigate_to {
            self.load_file(path);
            self.viewer.scroll_to_line = Some(line);
        }
    }

    /// Handle global keyboard shortcuts for search (called from update loop).
    pub(super) fn handle_search_shortcuts(&mut self, ctx: &egui::Context) {
        // Cmd+F = search in file
        if ctx.input(|i| i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::F))
        {
            if self.viewer.current_file.is_some() {
                self.search.in_file_active = true;
            }
        }

        // Cmd+Shift+F = search in files
        if ctx.input(|i| i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::F)) {
            self.search.in_files_active = true;
        }
    }
}
