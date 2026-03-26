/// An item fetched from an external source, to be converted to a Cue.
#[derive(Debug, Clone)]
pub(crate) struct SourceItem {
    pub external_id: String,
    pub text: String,
    pub source_label: String,
}

/// A finding extracted from a PR review comment.
#[derive(Debug, Clone)]
pub(crate) struct PrFinding {
    /// The file path the comment refers to (empty for general comments).
    pub file_path: String,
    /// The line number referenced (0 if not file-specific).
    pub line_number: usize,
    /// The finding text (reviewer comment body).
    pub text: String,
    /// A unique reference for deduplication (e.g. comment ID).
    pub external_id: String,
}
