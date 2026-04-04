/// An item fetched from an external source, to be converted to a Cue.
#[derive(Debug, Clone)]
pub(crate) struct SourceItem {
    pub external_id: String,
    pub text: String,
    pub source_label: String,
    /// Stable source identifier (matches SourceConfig.id).
    pub source_id: String,
    /// File path the finding refers to (empty for global/project-level items).
    pub file_path: String,
    /// Line number within the file (0 if not applicable).
    pub line_number: usize,
}

impl SourceItem {
    /// Create a new source item with `source_id` defaulting to empty.
    pub fn new(
        external_id: impl Into<String>,
        text: impl Into<String>,
        source_label: &str,
    ) -> Self {
        Self {
            external_id: external_id.into(),
            text: text.into(),
            source_label: source_label.to_string(),
            source_id: String::new(),
            file_path: String::new(),
            line_number: 0,
        }
    }

    /// Set the file location for this source item.
    pub fn with_location(mut self, file_path: &str, line_number: usize) -> Self {
        self.file_path = file_path.to_string();
        self.line_number = line_number;
        self
    }
}

/// A Notion database or page visible to the integration token.
#[derive(Debug, Clone)]
pub(crate) struct NotionObject {
    pub id: String,
    pub title: String,
    /// "database" or "page"
    pub object_type: String,
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
