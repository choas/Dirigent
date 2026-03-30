use std::path::{Path, PathBuf};

use crate::app::{symbols, DirigentApp};
use crate::file_tree::FileEntry;

impl DirigentApp {
    /// Try LSP go-to-definition first, fall back to regex-based search.
    pub(in crate::app) fn lsp_goto_definition(
        &mut self,
        file_path: &Path,
        line: u32,
        character: u32,
        word: &str,
    ) {
        if self.settings.lsp_enabled && self.lsp.has_initialized_server_for(file_path) {
            // Send LSP definition request — result will be polled in process_lsp_results
            self.lsp.definition(file_path, line, character);
            // Store fallback word so we can fall back to regex if LSP returns no result
            self.lsp_goto_def_fallback_word = Some(word.to_string());
            self.set_status_message(format!("LSP: looking up `{}`...", word));
        } else {
            self.goto_definition(word);
        }
    }

    /// Go to definition of a symbol: search project files for definition patterns.
    pub(in crate::app) fn goto_definition(&mut self, word: &str) {
        if word.is_empty() || word.len() < 2 {
            return;
        }

        let patterns = symbols::definition_patterns(word);
        if patterns.is_empty() {
            self.set_status_message(format!("No definition found for `{}`", word));
            return;
        }

        if self.search_definition_in_current_file(word, &patterns) {
            return;
        }

        self.spawn_definition_search(word, patterns);
    }

    /// Search for a definition in the current file. Returns true if found.
    fn search_definition_in_current_file(&mut self, word: &str, patterns: &[regex::Regex]) -> bool {
        let tab = match self.viewer.active() {
            Some(t) => t,
            None => return false,
        };
        let mut in_block_comment = false;
        for (idx, line) in tab.content.iter().enumerate() {
            let trimmed = line.trim();
            if is_comment_line(trimmed, &mut in_block_comment) {
                continue;
            }
            let found = patterns.iter().any(|re| re.is_match(line));
            if found {
                self.push_nav_history();
                self.viewer.scroll_to_line = Some(idx + 1);
                self.set_status_message(format!("Definition: `{}` at line {}", word, idx + 1));
                return true;
            }
        }
        false
    }

    /// Spawn a background thread to search for a symbol definition across project files.
    fn spawn_definition_search(&mut self, word: &str, patterns: Vec<regex::Regex>) {
        let mut all_files = Vec::new();
        if let Some(ref tree) = self.file_tree {
            crate::app::search::collect_files(&tree.entries, &mut all_files);
        }

        self.goto_def_cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.goto_def_cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.goto_def_gen = self.goto_def_gen.wrapping_add(1);
        let gen = self.goto_def_gen;

        let current_file = self.viewer.current_file().cloned();
        let project_root = self.project_root.clone();
        let word_owned = word.to_string();
        let tx = self.goto_def_tx.clone();
        let cancelled = self.goto_def_cancel.clone();

        self.set_status_message(format!("Searching for `{}`...", word));

        std::thread::spawn(move || {
            let result = search_files_for_definition(
                &all_files,
                &patterns,
                &cancelled,
                current_file.as_ref(),
                &project_root,
                &word_owned,
                gen,
            );
            let _ = tx.send(result.unwrap_or_else(|| {
                (
                    gen,
                    PathBuf::new(),
                    0,
                    format!("No definition found for `{}`", word_owned),
                )
            }));
        });
    }
}

/// Search project files for a definition in a background thread.
pub(crate) fn search_files_for_definition(
    all_files: &[PathBuf],
    patterns: &[regex::Regex],
    cancelled: &std::sync::atomic::AtomicBool,
    current_file: Option<&PathBuf>,
    project_root: &Path,
    word: &str,
    gen: u64,
) -> Option<(u64, PathBuf, usize, String)> {
    for file_path in all_files {
        if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
            return None;
        }
        if crate::app::search::is_binary_ext(file_path) {
            continue;
        }
        if current_file == Some(file_path) {
            continue;
        }
        let result =
            search_single_file_for_definition(file_path, patterns, project_root, word, gen);
        if result.is_some() {
            return result;
        }
    }
    None
}

/// Search a single file for a definition matching the given patterns.
fn search_single_file_for_definition(
    file_path: &Path,
    patterns: &[regex::Regex],
    project_root: &Path,
    word: &str,
    gen: u64,
) -> Option<(u64, PathBuf, usize, String)> {
    let content = std::fs::read_to_string(file_path).ok()?;
    let mut in_block_comment = false;
    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if is_comment_line(trimmed, &mut in_block_comment) {
            continue;
        }
        let found = patterns.iter().any(|re| re.is_match(line));
        if found {
            let target_line = idx + 1;
            let msg = format!(
                "Definition: `{}` at {}:{}",
                word,
                file_path
                    .strip_prefix(project_root)
                    .unwrap_or(file_path)
                    .display(),
                target_line
            );
            return Some((gen, file_path.to_path_buf(), target_line, msg));
        }
    }
    None
}

/// Delegate to the shared comment detector in symbols.
fn is_comment_line(trimmed: &str, in_block_comment: &mut bool) -> bool {
    symbols::is_comment_line(trimmed, in_block_comment)
}

/// Recursively collect all file paths with their relative paths.
pub(crate) fn collect_file_paths(
    entries: &[FileEntry],
    project_root: &std::path::Path,
    out: &mut Vec<(String, PathBuf)>,
) {
    for entry in entries {
        if entry.is_dir {
            collect_file_paths(&entry.children, project_root, out);
        } else {
            let rel = entry
                .path
                .strip_prefix(project_root)
                .unwrap_or(&entry.path)
                .to_string_lossy()
                .to_string();
            out.push((rel, entry.path.clone()));
        }
    }
}
