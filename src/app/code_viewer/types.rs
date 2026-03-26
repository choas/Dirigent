use std::collections::HashMap;

use crate::agents::Severity;

/// Highlight state for a single code line.
pub(crate) struct LineHighlight {
    pub in_selection: bool,
    pub current_search_match: bool,
    pub search_match: bool,
}

/// Accumulated actions from code line rendering, applied after the UI pass.
pub(crate) struct CodeLineActions {
    pub new_sel_start: Option<usize>,
    pub new_sel_end: Option<usize>,
    pub submit_cue: bool,
    pub clear_selection: bool,
    pub fix_diagnostic_line: Option<usize>,
    pub goto_def_word: Option<String>,
    pub implement_click_line: Option<usize>,
}

/// Per-render-pass context shared across all code lines.
pub(crate) struct CodeLineContext<'a> {
    pub active_idx: usize,
    pub sel_start: Option<usize>,
    pub sel_end: Option<usize>,
    pub lines_with_cues: &'a HashMap<usize, bool>,
    pub diag_lines: &'a HashMap<usize, Severity>,
    pub diag_messages: &'a HashMap<usize, Vec<String>>,
    pub ext: &'a str,
    pub symbol_lines: &'a HashMap<usize, (String, String)>,
    pub cmd_held: bool,
}

/// Result of tab bar rendering: what action, if any, to apply.
pub(crate) enum TabBarAction {
    None,
    CloseAll,
    CloseOthers(usize),
    CloseToRight(usize),
    CloseOne(usize),
    Activate(usize),
}

/// Result of breadcrumb bar rendering: what action, if any, to apply.
pub(crate) enum BreadcrumbAction {
    None,
    CloseFile,
    ShowFileDiff,
    ToggleMarkdown,
}
