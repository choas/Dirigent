use std::path::{Path, PathBuf};

use git2::Repository;

/// Entries that are always hidden from the file tree (not useful to display).
const ALWAYS_HIDDEN: &[&str] = &[".git", ".DS_Store"];

#[derive(Debug, Clone)]
pub(crate) struct FileEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub is_ignored: bool,
    pub children: Vec<FileEntry>,
}

#[derive(Debug, Clone)]
pub(crate) struct FileTree {
    #[allow(dead_code)]
    pub root: PathBuf,
    pub entries: Vec<FileEntry>,
}

impl FileTree {
    pub fn scan(root: &Path) -> std::io::Result<Self> {
        let repo = Repository::discover(root).ok();
        let entries = scan_directory(root, root, repo.as_ref())?;
        Ok(FileTree {
            root: root.to_path_buf(),
            entries,
        })
    }
}

fn scan_directory(
    dir: &Path,
    root: &Path,
    repo: Option<&Repository>,
) -> std::io::Result<Vec<FileEntry>> {
    let mut entries = Vec::new();

    let read_dir = std::fs::read_dir(dir)?;
    for entry in read_dir {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();

        if ALWAYS_HIDDEN.iter().any(|p| *p == name) {
            continue;
        }

        let path = entry.path();
        let file_type = entry.file_type()?;
        let is_dir = file_type.is_dir();

        let is_ignored = repo
            .and_then(|r| {
                let rel = path.strip_prefix(root).ok()?;
                r.is_path_ignored(rel).ok()
            })
            .unwrap_or(false);

        let children = if is_dir {
            scan_directory(&path, root, repo)?
        } else {
            Vec::new()
        };

        entries.push(FileEntry {
            name,
            path,
            is_dir,
            is_ignored,
            children,
        });
    }

    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    Ok(entries)
}
