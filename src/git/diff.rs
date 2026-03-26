use std::collections::HashSet;
use std::path::Path;

use git2::{Repository, StatusOptions};

/// Get the working-tree diff for specific files (or all files if empty).
/// Returns the `git diff` output, or None if there are no changes.
/// Also generates diffs for untracked (new) files so they appear in review and commits.
pub(crate) fn get_working_diff(repo_path: &Path, files: &[String]) -> Option<String> {
    use std::process::Command;

    let rel_files: Vec<String> = files
        .iter()
        .map(|f| make_path_relative(repo_path, f))
        .collect();

    // Get diff for tracked/modified files
    let mut cmd = Command::new("git");
    cmd.arg("diff").current_dir(repo_path);
    if !rel_files.is_empty() {
        cmd.arg("--");
        for f in &rel_files {
            cmd.arg(f);
        }
    }

    let output = cmd.output().ok()?;
    let mut diff = if output.status.success() {
        String::from_utf8_lossy(&output.stdout).to_string()
    } else {
        String::new()
    };

    // Find untracked files and generate new-file diffs for them
    let untracked = find_untracked_files(repo_path);
    let check_files: Vec<&String> = if rel_files.is_empty() {
        untracked.iter().collect()
    } else {
        rel_files
            .iter()
            .filter(|f| untracked.contains(f.as_str()))
            .collect()
    };

    for rel_path in check_files {
        append_new_file_diff(&mut diff, repo_path, rel_path);
    }

    if diff.trim().is_empty() {
        None
    } else {
        Some(diff)
    }
}

fn make_path_relative(repo_path: &Path, f: &str) -> String {
    let path = Path::new(f);
    if path.is_absolute() {
        path.strip_prefix(repo_path)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string()
    } else {
        f.to_string()
    }
}

fn append_new_file_diff(diff: &mut String, repo_path: &Path, rel_path: &str) {
    let full_path = repo_path.join(rel_path);
    let contents = match std::fs::read_to_string(&full_path) {
        Ok(c) => c,
        Err(_) => return,
    };
    let line_count = contents.lines().count();
    diff.push_str(&format!("diff --git a/{rel_path} b/{rel_path}\n"));
    diff.push_str("new file mode 100644\n");
    diff.push_str("--- /dev/null\n");
    diff.push_str(&format!("+++ b/{rel_path}\n"));
    if line_count == 0 {
        diff.push_str("@@ -0,0 +0,0 @@\n");
    } else {
        diff.push_str(&format!("@@ -0,0 +1,{line_count} @@\n"));
        for line in contents.lines() {
            diff.push('+');
            diff.push_str(line);
            diff.push('\n');
        }
        if !contents.ends_with('\n') {
            diff.push_str("\\ No newline at end of file\n");
        }
    }
}

/// Returns relative paths of all untracked files in the repo.
fn find_untracked_files(repo_path: &Path) -> HashSet<String> {
    let mut result = HashSet::new();
    let repo = match Repository::discover(repo_path) {
        Ok(r) => r,
        Err(_) => return result,
    };
    let mut opts = StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);
    if let Ok(statuses) = repo.statuses(Some(&mut opts)) {
        for entry in statuses.iter() {
            let s = entry.status();
            if s.intersects(git2::Status::WT_NEW) && !s.intersects(git2::Status::INDEX_NEW) {
                if let Some(p) = entry.path() {
                    result.insert(p.to_string());
                }
            }
        }
    }
    result
}

/// Like parse_diff_file_paths but also strips a directory prefix if present.
/// Use this when the diff may have been generated with paths relative to a parent dir.
pub(crate) fn parse_diff_file_paths_for_repo(repo_path: &Path, diff_text: &str) -> Vec<String> {
    let dir_name = repo_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let dir_prefix = if dir_name.is_empty() {
        String::new()
    } else {
        format!("{}/", dir_name)
    };

    let mut paths = Vec::new();
    for line in diff_text.lines() {
        let rest = line
            .strip_prefix("+++ b/")
            .or_else(|| line.strip_prefix("--- a/"));
        let rest = match rest {
            Some(r) => r,
            None => continue,
        };
        let path = rest.trim();
        if path.is_empty() {
            continue;
        }
        // Strip dir prefix if present
        let path = path.strip_prefix(dir_prefix.as_str()).unwrap_or(path);
        let path = path.to_string();
        if !path.is_empty() && !paths.contains(&path) {
            paths.push(path);
        }
    }
    paths
}

/// Parse file paths from diff text, stripping the repo directory prefix if present.
pub(super) fn parse_diff_paths(repo_path: &Path, diff_text: &str) -> Vec<String> {
    let dir_name = repo_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let dir_prefix = if dir_name.is_empty() {
        String::new()
    } else {
        format!("{}/", dir_name)
    };

    let mut file_paths = Vec::new();
    for line in diff_text.lines() {
        let rest = line
            .strip_prefix("+++ b/")
            .or_else(|| line.strip_prefix("--- a/"));
        let rest = match rest {
            Some(r) => r,
            None => continue,
        };
        let path = rest.trim();
        let path = path.strip_prefix(dir_prefix.as_str()).unwrap_or(path);
        let path = path.to_string();
        if !path.is_empty() && !file_paths.contains(&path) {
            file_paths.push(path);
        }
    }
    file_paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_diff_file_paths_simple() {
        let diff = "\
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,3 @@
 fn main() {}
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,1 +1,1 @@
-old
+new
";
        let paths = parse_diff_file_paths_for_repo(Path::new("/tmp/myproject"), diff);
        assert_eq!(paths, vec!["src/main.rs", "src/lib.rs"]);
    }

    #[test]
    fn parse_diff_file_paths_strips_repo_prefix() {
        let diff = "\
--- a/myproject/src/app.rs
+++ b/myproject/src/app.rs
@@ -1,1 +1,1 @@
-x
+y
";
        let paths = parse_diff_file_paths_for_repo(Path::new("/home/user/myproject"), diff);
        assert_eq!(paths, vec!["src/app.rs"]);
    }

    #[test]
    fn parse_diff_file_paths_no_duplicates() {
        let diff = "\
--- a/f.rs
+++ b/f.rs
@@ -1,1 +1,1 @@
-a
+b
--- a/f.rs
+++ b/f.rs
@@ -10,1 +10,1 @@
-c
+d
";
        let paths = parse_diff_file_paths_for_repo(Path::new("/tmp/proj"), diff);
        assert_eq!(paths, vec!["f.rs"]);
    }

    #[test]
    fn parse_diff_file_paths_empty_diff() {
        let paths = parse_diff_file_paths_for_repo(Path::new("/tmp/proj"), "");
        assert!(paths.is_empty());
    }
}
