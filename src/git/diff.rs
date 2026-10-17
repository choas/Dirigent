use std::collections::HashSet;
use std::path::Path;

use git2::Repository;

/// Get the working-tree diff for specific files (or all files if empty).
/// Returns the `git diff` output, or None if there are no changes.
/// Also generates diffs for untracked (new) files so they appear in review and commits.
pub(crate) fn get_working_diff(repo_path: &Path, files: &[String]) -> Option<String> {
    diff_with_untracked(repo_path, files, false)
}

/// Diff of everything uncommitted — staged plus unstaged changes — relative to
/// HEAD (`git diff HEAD`), including untracked files. Unlike
/// [`get_working_diff`] this still reports files whose changes are fully
/// staged, so commit flows keep working after a `git add`. Falls back to the
/// index-relative diff on an unborn branch, where HEAD has no commit.
pub(crate) fn get_uncommitted_diff(repo_path: &Path, files: &[String]) -> Option<String> {
    diff_with_untracked(repo_path, files, true)
}

fn diff_with_untracked(repo_path: &Path, files: &[String], vs_head: bool) -> Option<String> {
    let rel_files: Vec<String> = files
        .iter()
        .map(|f| make_path_relative(repo_path, f))
        .collect();

    // Get diff for tracked/modified files
    let mut diff = if vs_head {
        tracked_diff(repo_path, &rel_files, true)
            // Unborn branch: no HEAD commit to diff against.
            .or_else(|| tracked_diff(repo_path, &rel_files, false))
    } else {
        tracked_diff(repo_path, &rel_files, false)
    }
    .unwrap_or_default();

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

/// Run `git diff` (optionally against HEAD) scoped to `rel_files`.
/// Returns `None` when the command fails (e.g. `HEAD` on an unborn branch).
fn tracked_diff(repo_path: &Path, rel_files: &[String], vs_head: bool) -> Option<String> {
    let mut cmd = super::git_cmd();
    cmd.arg("diff").current_dir(repo_path);
    if vs_head {
        cmd.arg("HEAD");
    }
    if !rel_files.is_empty() {
        cmd.arg("--");
        for f in rel_files {
            cmd.arg(f);
        }
    }
    let output = cmd.output().ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        None
    }
}

/// Diff between two refs using merge-base (three-dot) semantics: `git diff
/// base...head`. This shows what `head` introduced relative to their common
/// ancestor — the natural "what's new on this branch" view for review — rather
/// than every difference between the two tips. Returns `None` when the refs are
/// identical or produce no differences.
#[cfg(test)]
pub(crate) fn get_range_diff(repo_path: &Path, base: &str, head: &str) -> Option<String> {
    try_get_range_diff(repo_path, base, head).ok().flatten()
}

/// Like [`get_range_diff`], but distinguishes "no differences" (`Ok(None)`) from
/// "git could not answer" (`Err`).
///
/// Collapsing both into `None` surfaced a bad ref, a missing remote branch or a
/// corrupt repository to the user as the cheerful "Nothing to analyze between
/// those refs" — which is exactly the wrong thing to tell someone who typo'd a
/// branch name.
pub(crate) fn try_get_range_diff(
    repo_path: &Path,
    base: &str,
    head: &str,
) -> Result<Option<String>, String> {
    let range = format!("{base}...{head}");
    let output = super::git_cmd()
        .arg("diff")
        .arg(&range)
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("could not run git diff {range}: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.trim();
        return Err(if detail.is_empty() {
            format!("git diff {range} failed")
        } else {
            format!("git diff {range} failed: {detail}")
        });
    }
    let diff = String::from_utf8_lossy(&output.stdout).to_string();
    if diff.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(diff))
    }
}

/// Result of comparing two folders.
pub(crate) enum FolderCompare {
    /// Folders differ; carries the unified diff.
    Diff(String),
    /// Folders are identical.
    Identical,
}

/// Compare two folders with `git diff --no-index left right` (works outside any
/// repo). `git diff --no-index` exits 0 when identical, 1 when they differ, and
/// >1 on error — so we map those to `Identical` / `Diff` / an error.
pub(crate) fn compare_folders(left: &Path, right: &Path) -> crate::error::Result<FolderCompare> {
    use crate::error::DirigentError;

    for p in [left, right] {
        if !p.exists() {
            return Err(DirigentError::GitCommand(format!(
                "path does not exist: {}",
                p.display()
            )));
        }
    }

    let output = super::git_cmd()
        .arg("diff")
        .arg("--no-index")
        .arg(left)
        .arg(right)
        .output()?;

    match output.status.code() {
        Some(0) => Ok(FolderCompare::Identical),
        Some(1) => {
            let diff = String::from_utf8_lossy(&output.stdout).into_owned();
            if diff.trim().is_empty() {
                Ok(FolderCompare::Identical)
            } else {
                Ok(FolderCompare::Diff(diff))
            }
        }
        _ => Err(DirigentError::GitCommand(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )),
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
    if let Some(statuses) = super::collect_statuses(&repo) {
        for entry in statuses.iter() {
            let s = entry.status();
            if s.intersects(git2::Status::WT_NEW)
                && !s.intersects(git2::Status::INDEX_NEW)
                && let Some(p) = entry.path()
            {
                result.insert(p.to_string());
            }
        }
    }
    result
}

/// Like parse_diff_file_paths but also strips a directory prefix if present.
/// Use this when the diff may have been generated with paths relative to a parent dir.
pub(crate) fn parse_diff_file_paths_for_repo(repo_path: &Path, diff_text: &str) -> Vec<String> {
    parse_diff_paths_with_repo_prefix(repo_path, diff_text)
}

/// Parse file paths from diff text, stripping the repo directory prefix if present.
pub(super) fn parse_diff_paths(repo_path: &Path, diff_text: &str) -> Vec<String> {
    parse_diff_paths_with_repo_prefix(repo_path, diff_text)
}

/// Stateful scan for `--- a/` / `+++ b/` file headers.
///
/// The scan tracks each hunk's line counts from its `@@` header so hunk
/// *content* is never mistaken for a file header: a deleted source line whose
/// content begins with `-- ` renders as `--- ...` in the diff, and a stateless
/// prefix match parsed it as a path — commit flows then staged and committed
/// an unrelated file. While a hunk's budget remains, every line is consumed as
/// content; headers are only recognised between hunks, and a `diff --git`
/// line always ends the current hunk.
fn parse_diff_paths_with_repo_prefix(repo_path: &Path, diff_text: &str) -> Vec<String> {
    let dir_name = repo_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let dir_prefix = repo_dir_prefix(&dir_name);
    let mut file_paths = Vec::new();
    // Remaining (old, new) lines of the hunk currently being scanned.
    let mut budget: Option<(usize, usize)> = None;
    for line in diff_text.lines() {
        if line.starts_with("diff --git ") {
            budget = None;
            continue;
        }
        if let Some((old_left, new_left)) = budget {
            if let Some(rest) = consume_hunk_line(line, old_left, new_left) {
                budget = (rest != (0, 0)).then_some(rest);
                continue;
            }
            // Not valid hunk content — the header's counts were short.
            // Fall through and scan the line normally.
            budget = None;
        }
        if line.starts_with("@@ ") {
            let counts = crate::claude::parse_hunk_header_counts(line);
            budget = (counts != (0, 0)).then_some(counts);
            continue;
        }
        let Some(path) = diff_line_path(repo_path, &dir_prefix, line) else {
            continue;
        };
        if !file_paths.contains(&path) {
            file_paths.push(path);
        }
    }
    file_paths
}

/// Consume one hunk-body line against the remaining `(old, new)` budget.
/// Returns the budget left afterwards, or `None` when the line cannot be hunk
/// content given that budget.
fn consume_hunk_line(line: &str, old_left: usize, new_left: usize) -> Option<(usize, usize)> {
    match line.as_bytes().first() {
        Some(b'\\') => Some((old_left, new_left)), // "\ No newline at end of file"
        Some(b'+') if new_left > 0 => Some((old_left, new_left - 1)),
        Some(b'-') if old_left > 0 => Some((old_left - 1, new_left)),
        Some(b' ') | None if old_left > 0 && new_left > 0 => Some((old_left - 1, new_left - 1)),
        _ => None,
    }
}

fn repo_dir_prefix(dir_name: &str) -> String {
    if dir_name.is_empty() {
        String::new()
    } else {
        format!("{}/", dir_name)
    }
}

fn diff_line_path(repo_path: &Path, dir_prefix: &str, line: &str) -> Option<String> {
    let path = line
        .strip_prefix("+++ b/")
        .or_else(|| line.strip_prefix("--- a/"))?
        .trim();
    if path.is_empty() {
        return None;
    }
    Some(strip_spurious_repo_prefix(repo_path, dir_prefix, path).to_string())
}

fn strip_spurious_repo_prefix<'a>(repo_path: &Path, dir_prefix: &str, path: &'a str) -> &'a str {
    if dir_prefix.is_empty() || repo_path.join(path).exists() {
        return path;
    }
    path.strip_prefix(dir_prefix).unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(dir: &Path, args: &[&str]) {
        let out = crate::git::git_cmd()
            .current_dir(dir)
            .args(args)
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {:?}: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[test]
    fn compare_folders_differ_identical_and_missing() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        let b = d.path().join("b");
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&b).unwrap();
        std::fs::write(a.join("f.txt"), "one\n").unwrap();
        std::fs::write(b.join("f.txt"), "two\n").unwrap();

        // Differing folders -> a diff mentioning the changed lines and paths.
        match compare_folders(&a, &b).unwrap() {
            FolderCompare::Diff(diff) => {
                assert!(diff.contains("-one"));
                assert!(diff.contains("+two"));
                let paths = parse_diff_file_paths_for_repo(d.path(), &diff);
                assert!(!paths.is_empty(), "diff should parse into file paths");
            }
            FolderCompare::Identical => panic!("folders differ"),
        }

        // Identical content -> Identical, not an error.
        std::fs::write(b.join("f.txt"), "one\n").unwrap();
        assert!(matches!(
            compare_folders(&a, &b).unwrap(),
            FolderCompare::Identical
        ));

        // Missing path -> error.
        assert!(compare_folders(&a, &d.path().join("nope")).is_err());
    }

    #[test]
    fn range_diff_diverged_vs_identical() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        run(p, &["init", "-q"]);
        run(p, &["config", "user.email", "t@t.t"]);
        run(p, &["config", "user.name", "T"]);
        std::fs::write(p.join("f.txt"), "a\n").unwrap();
        run(p, &["add", "."]);
        run(p, &["commit", "-qm", "base"]);
        run(p, &["branch", "feature"]);
        run(p, &["checkout", "-q", "feature"]);
        std::fs::write(p.join("f.txt"), "a\nb\n").unwrap();
        run(p, &["commit", "-qam", "feature work"]);

        // Diverged: main...feature shows the feature's addition.
        let d = get_range_diff(p, "master", "feature")
            .or_else(|| get_range_diff(p, "main", "feature"))
            .expect("diverged refs produce a diff");
        assert!(d.contains("+b"));
        // Identical refs -> nothing.
        assert!(get_range_diff(p, "feature", "feature").is_none());
    }

    #[test]
    fn parse_diff_file_paths_simple() {
        let diff = "\
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,1 +1,1 @@
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

    #[test]
    fn parse_diff_file_paths_ignores_header_lookalikes_inside_hunks() {
        // Deleting a SQL/Lua-style `-- ` comment renders as `--- ...` in the
        // diff body. A stateless prefix match parsed it as a file header, and
        // commit_diff then staged and committed the unrelated path.
        let diff = "\
diff --git a/src/query.sql b/src/query.sql
--- a/src/query.sql
+++ b/src/query.sql
@@ -1,3 +1,3 @@
 SELECT 1;
--- a/unrelated_file.rs
+-- new comment
 SELECT 2;
";
        let paths = parse_diff_file_paths_for_repo(Path::new("/tmp/proj"), diff);
        assert_eq!(paths, vec!["src/query.sql"]);
    }

    #[test]
    fn parse_diff_file_paths_ignores_plus_plus_content_lines() {
        // An added line whose content starts with `++ ` renders as `+++ ...`.
        let diff = "\
diff --git a/inc.cc b/inc.cc
--- a/inc.cc
+++ b/inc.cc
@@ -1,2 +1,3 @@
 int i = 0;
+++ b/evil.rs
 int j = 1;
";
        let paths = parse_diff_file_paths_for_repo(Path::new("/tmp/proj"), diff);
        assert_eq!(paths, vec!["inc.cc"]);
    }

    #[test]
    fn parse_diff_file_paths_header_pair_lookalike_in_hunk() {
        // A deleted `-- a/x` plus an added `++ b/y` reproduce an exact
        // `--- a/` / `+++ b/` header pair inside the hunk body.
        let diff = "\
diff --git a/notes.md b/notes.md
--- a/notes.md
+++ b/notes.md
@@ -1,2 +1,2 @@
--- a/phantom.rs
+++ b/phantom.rs
 tail
";
        let paths = parse_diff_file_paths_for_repo(Path::new("/tmp/proj"), diff);
        assert_eq!(paths, vec!["notes.md"]);
    }

    #[test]
    fn parse_diff_file_paths_second_file_after_lookalike_hunk() {
        // Once the hunk's line budget is exhausted, a genuine header for the
        // next file must still be recognised — even without `diff --git`
        // anchors (Claude-produced diffs lack them).
        let diff = "\
--- a/first.sql
+++ b/first.sql
@@ -1,2 +1,1 @@
--- old comment
 SELECT 1;
--- a/second.rs
+++ b/second.rs
@@ -1,1 +1,1 @@
-x
+y
";
        let paths = parse_diff_file_paths_for_repo(Path::new("/tmp/proj"), diff);
        assert_eq!(paths, vec!["first.sql", "second.rs"]);
    }
}
