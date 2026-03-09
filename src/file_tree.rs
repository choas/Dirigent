use std::path::{Path, PathBuf};

const DEFAULT_IGNORE_PATTERNS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    ".DS_Store",
    ".Dirigent",
    ".ralph",
    "_build",
    "deps",
];

#[derive(Debug, Clone)]
pub(crate) struct FileEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub children: Vec<FileEntry>,
}

#[derive(Debug, Clone)]
pub(crate) struct FileTree {
    pub root: PathBuf,
    pub entries: Vec<FileEntry>,
}

impl FileTree {
    pub fn scan(root: &Path) -> std::io::Result<Self> {
        let entries = scan_directory(root, DEFAULT_IGNORE_PATTERNS)?;
        Ok(FileTree {
            root: root.to_path_buf(),
            entries,
        })
    }
}

fn scan_directory(dir: &Path, ignore_patterns: &[&str]) -> std::io::Result<Vec<FileEntry>> {
    let mut entries = Vec::new();

    let read_dir = std::fs::read_dir(dir)?;
    for entry in read_dir {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();

        if ignore_patterns.iter().any(|p| *p == name) {
            continue;
        }

        let path = entry.path();
        let file_type = entry.file_type()?;
        let is_dir = file_type.is_dir();

        let children = if is_dir {
            scan_directory(&path, ignore_patterns)?
        } else {
            Vec::new()
        };

        entries.push(FileEntry {
            name,
            path,
            is_dir,
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
