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
        let child = entry
            .children
            .into_iter()
            .next()
            .expect("children.len() == 1 guarantees an element");
        entry.name = format!("{}/{}", entry.name, child.name);
        entry.path = child.path;
        entry.is_ignored = entry.is_ignored || child.is_ignored;
        entry.children = child.children;
    }
    // If the collapsed directory contains exactly one file (no subdirs),
    // flatten it into a single file entry with the combined path as name.
    if entry.children.len() == 1 && !entry.children[0].is_dir {
        let child = entry
            .children
            .into_iter()
            .next()
            .expect("children.len() == 1 guarantees an element");
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

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str, children: Vec<FileEntry>) -> FileEntry {
        FileEntry {
            name: name.into(),
            path: PathBuf::from(name),
            is_dir: true,
            is_ignored: false,
            children,
        }
    }

    fn file(name: &str) -> FileEntry {
        FileEntry {
            name: name.into(),
            path: PathBuf::from(name),
            is_dir: false,
            is_ignored: false,
            children: Vec::new(),
        }
    }

    #[test]
    fn collapse_single_child_chain() {
        // src -> main -> java -> App.java  =>  src/main/java/App.java (file)
        let tree = vec![dir(
            "src",
            vec![dir("main", vec![dir("java", vec![file("App.java")])])],
        )];
        let collapsed = collapse_single_child_dirs(tree);
        assert_eq!(collapsed.len(), 1);
        assert_eq!(collapsed[0].name, "src/main/java/App.java");
        assert!(!collapsed[0].is_dir);
    }

    #[test]
    fn collapse_stops_at_multiple_children() {
        // src -> main -> {Foo.rs, Bar.rs}
        let tree = vec![dir(
            "src",
            vec![dir("main", vec![file("Foo.rs"), file("Bar.rs")])],
        )];
        let collapsed = collapse_single_child_dirs(tree);
        assert_eq!(collapsed.len(), 1);
        assert_eq!(collapsed[0].name, "src/main");
        assert!(collapsed[0].is_dir);
        assert_eq!(collapsed[0].children.len(), 2);
    }

    #[test]
    fn collapse_preserves_flat_structure() {
        let tree = vec![file("README.md"), file("Cargo.toml")];
        let collapsed = collapse_single_child_dirs(tree);
        assert_eq!(collapsed.len(), 2);
        assert_eq!(collapsed[0].name, "README.md");
    }

    #[test]
    fn collapse_propagates_ignored_flag() {
        let mut parent = dir("vendor", vec![dir("lib", vec![file("x.js")])]);
        parent.is_ignored = true;
        let collapsed = collapse_single_child_dirs(vec![parent]);
        assert!(collapsed[0].is_ignored);
    }

    #[test]
    fn collapse_file_is_identity() {
        let f = file("hello.txt");
        let result = collapse_entry(f);
        assert_eq!(result.name, "hello.txt");
        assert!(!result.is_dir);
    }

    #[test]
    fn collapse_dir_with_mixed_children() {
        // src -> {lib (dir with files), main.rs (file)}
        let tree = vec![dir(
            "src",
            vec![
                dir("lib", vec![file("a.rs"), file("b.rs")]),
                file("main.rs"),
            ],
        )];
        let collapsed = collapse_single_child_dirs(tree);
        assert_eq!(collapsed[0].name, "src");
        assert!(collapsed[0].is_dir);
        assert_eq!(collapsed[0].children.len(), 2);
    }
}
