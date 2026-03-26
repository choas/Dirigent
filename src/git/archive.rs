use std::path::{Path, PathBuf};

use crate::error::DirigentError;

/// Archives the worktree's .Dirigent/Dirigent.db to the main repo's
/// .Dirigent/archives/<branch-name>.db before removal.
/// Returns Ok(Some(archive_path)) if archived, Ok(None) if no DB existed.
pub(crate) fn archive_worktree_db(
    main_repo_path: &Path,
    worktree_path: &Path,
    worktree_name: &str,
) -> crate::error::Result<Option<PathBuf>> {
    let src_db = worktree_path.join(".Dirigent").join("Dirigent.db");
    if !src_db.exists() {
        return Ok(None);
    }

    // Checkpoint the WAL so all data is flushed into the main DB file before copying.
    // TRUNCATE mode also removes the -wal and -shm files afterward.
    {
        let conn = rusqlite::Connection::open_with_flags(
            &src_db,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
        )
        .map_err(|e| {
            DirigentError::Sqlite(format!(
                "failed to open worktree DB for WAL checkpoint: {}",
                e
            ))
        })?;
        conn.pragma_update(None, "wal_checkpoint", "TRUNCATE")
            .map_err(|e| DirigentError::Sqlite(format!("WAL checkpoint failed: {}", e)))?;
    }

    let archives_dir = main_repo_path.join(".Dirigent").join("archives");
    std::fs::create_dir_all(&archives_dir)
        .map_err(|e| DirigentError::GitCommand(format!("failed to create archives dir: {}", e)))?;

    // Sanitize worktree name for cross-platform filename safety:
    // replace path separators, Windows-invalid chars, and control characters.
    let safe_name: String = worktree_name
        .chars()
        .map(|c| {
            if c == '/'
                || c == '\\'
                || matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|')
                || c.is_control()
            {
                '-'
            } else {
                c
            }
        })
        .collect();
    // Collapse consecutive dashes, trim leading/trailing dashes and dots
    let safe_name: String = safe_name
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let safe_name = safe_name.trim_matches('.').to_string();
    // Fallback if the result is empty
    let safe_name = if safe_name.is_empty() {
        format!("worktree_{}", chrono::Utc::now().format("%Y%m%dT%H%M%S"))
    } else {
        safe_name
    };

    let mut target = archives_dir.join(format!("{}.db", safe_name));
    if target.exists() {
        // Append UTC timestamp to avoid collision
        let now = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S");
        target = archives_dir.join(format!("{}_{}.db", safe_name, now));
    }

    std::fs::copy(&src_db, &target)
        .map_err(|e| DirigentError::GitCommand(format!("failed to archive worktree DB: {}", e)))?;

    Ok(Some(target))
}

/// Archived worktree DB entry.
#[derive(Debug, Clone)]
pub(crate) struct ArchivedDb {
    pub name: String,
    pub path: PathBuf,
    pub size_bytes: u64,
    pub modified: std::time::SystemTime,
}

/// Lists all archived worktree DBs in <main_repo>/.Dirigent/archives/.
pub(crate) fn list_archived_dbs(main_repo_path: &Path) -> Vec<ArchivedDb> {
    let archives_dir = main_repo_path.join(".Dirigent").join("archives");
    let entries = match std::fs::read_dir(&archives_dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!(
                "failed to read archives dir {}: {e}",
                archives_dir.display()
            );
            return Vec::new();
        }
    };

    let mut result = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("db") {
            if let Ok(meta) = entry.metadata() {
                let name = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                let modified = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
                result.push(ArchivedDb {
                    name,
                    path,
                    size_bytes: meta.len(),
                    modified,
                });
            }
        }
    }
    // Sort by modified time, newest first
    result.sort_by(|a, b| b.modified.cmp(&a.modified));
    result
}
