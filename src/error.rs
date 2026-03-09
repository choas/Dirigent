use thiserror::Error;

#[derive(Debug, Error)]
pub enum DirigentError {
    #[error("database error: {0}")]
    Db(#[from] anyhow::Error),

    #[error("git error: {0}")]
    Git(#[from] git2::Error),

    #[error("git: {0}")]
    GitCommand(String),

    #[error("claude error: {0}")]
    Claude(#[from] crate::claude::ClaudeError),

    #[error("source: {0}")]
    Source(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, DirigentError>;
