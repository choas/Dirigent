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
    pub entries: Vec<FileEntry>,
}

impl FileTree {
    pub fn scan(root: &Path) -> std::io::Result<Self> {
        let repo = Repository::discover(root).ok();
        let entries = scan_directory(root, root, repo.as_ref())?;
        let entries = collapse_single_child_dirs(entries);
        Ok(FileTree { entries })
    }
}

/// Collapse chains of directories that contain only a single subdirectory
/// (and no files) into one entry with a combined name like `src/main/java`.
fn collapse_single_child_dirs(entries: Vec<FileEntry>) -> Vec<FileEntry> {
    entries.into_iter().map(collapse_entry).collect()
}

fn collapse_entry(mut entry: FileEntry) -> FileEntry {
    if !entry.is_dir {
        return entry;
    }
    // While this directory has exactly one child and that child is a directory,
    // absorb it into the current entry's display name.
    while entry.children.len() == 1 && entry.children[0].is_dir {
        let child = entry.children.into_iter().next().unwrap();
        entry.name = format!("{}/{}", entry.name, child.name);
        entry.path = child.path;
        entry.is_ignored = entry.is_ignored || child.is_ignored;
        entry.children = child.children;
    }
    // If the collapsed directory contains exactly one file (no subdirs),
    // flatten it into a single file entry with the combined path as name.
    if entry.children.len() == 1 && !entry.children[0].is_dir {
        let child = entry.children.into_iter().next().unwrap();
        return FileEntry {
            name: format!("{}/{}", entry.name, child.name),
            path: child.path,
            is_dir: false,
            is_ignored: entry.is_ignored || child.is_ignored,
            children: Vec::new(),
        };
    }
    // Recursively collapse remaining children
    entry.children = collapse_single_child_dirs(entry.children);
    entry
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
