//! Hunk-level (partial) staging via `git apply` patches.
//!
//! Single hunks are staged/unstaged/discarded by feeding `git apply` a textual
//! one-hunk unified-diff patch — the same mechanism `git add -p` uses. The patch
//! is sliced **verbatim** out of the raw `git diff` output rather than rebuilt
//! from Dirigent's parsed diff model, which is lossy (it drops `@@` lengths,
//! `\ No newline at end of file` markers, and mode/new-file headers). Keeping the
//! exact bytes is what makes `git apply` accept the patch.
//!
//! `git apply` is atomic per invocation: a patch that doesn't apply cleanly is
//! rejected wholesale, giving us the failure isolation we want.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::error::{DirigentError, Result};

/// One file's section of a raw unified diff, retained verbatim so single-hunk
/// patches can be reconstructed losslessly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileRawDiff {
    /// The file's path (new path, or old path for a deletion).
    pub path: String,
    /// Header lines from `diff --git` up to (excluding) the first `@@` hunk.
    header: Vec<String>,
    /// Each hunk as its verbatim lines, starting with the `@@ ... @@` line.
    hunks: Vec<Vec<String>>,
    /// True for a binary file (no textual hunks; whole-file staging only).
    pub binary: bool,
}

impl FileRawDiff {
    pub fn hunk_count(&self) -> usize {
        self.hunks.len()
    }
}

/// Split a raw (possibly multi-file) unified diff into per-file sections,
/// preserving every line verbatim.
pub(crate) fn split_into_file_diffs(diff: &str) -> Vec<FileRawDiff> {
    let mut files: Vec<FileRawDiff> = Vec::new();
    let mut cur: Option<FileRawDiff> = None;
    // Whether we've passed the header and are accumulating hunk bodies.
    let mut in_hunks = false;

    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            if let Some(f) = cur.take() {
                files.push(f);
            }
            cur = Some(FileRawDiff {
                path: parse_diff_git_path(line),
                header: vec![line.to_string()],
                hunks: Vec::new(),
                binary: false,
            });
            in_hunks = false;
            continue;
        }
        let Some(f) = cur.as_mut() else {
            // Lines before the first `diff --git` (shouldn't happen) are ignored.
            continue;
        };
        if line.starts_with("@@ ") {
            in_hunks = true;
            f.hunks.push(vec![line.to_string()]);
        } else if in_hunks {
            // Body line of the current hunk (context/+/-/\ marker).
            if let Some(h) = f.hunks.last_mut() {
                h.push(line.to_string());
            }
        } else {
            // Still in the file header block.
            if line.starts_with("Binary files ") || line.starts_with("GIT binary patch") {
                f.binary = true;
            }
            // Prefer the explicit +++/--- paths when present.
            if let Some(p) = line.strip_prefix("+++ ") {
                if p != "/dev/null" {
                    f.path = strip_ab_prefix(p);
                }
            } else if let Some(p) = line.strip_prefix("--- ") {
                if p != "/dev/null" && f.path.is_empty() {
                    f.path = strip_ab_prefix(p);
                }
            }
            f.header.push(line.to_string());
        }
    }
    if let Some(f) = cur.take() {
        files.push(f);
    }
    files
}

/// Extract the path from a `diff --git a/<path> b/<path>` line (the `b/` side).
fn parse_diff_git_path(line: &str) -> String {
    // Format: `diff --git a/<old> b/<new>`. Take the `b/` side when present.
    if let Some(idx) = line.find(" b/") {
        return line[idx + 3..].trim().to_string();
    }
    String::new()
}

/// Strip a leading `a/` or `b/` (or a timestamp suffix) from a `---`/`+++` path.
fn strip_ab_prefix(p: &str) -> String {
    // `+++ b/path\t<timestamp>` — keep only the path portion.
    let path = p.split('\t').next().unwrap_or(p).trim();
    path.strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path)
        .to_string()
}

/// Build a valid single-hunk unified-diff patch: the file header block plus
/// exactly one `@@` hunk, each line terminated with `\n`. Returns `None` when
/// the index is out of range or the file is binary.
pub(crate) fn build_hunk_patch(file: &FileRawDiff, hunk_idx: usize) -> Option<String> {
    if file.binary {
        return None;
    }
    let hunk = file.hunks.get(hunk_idx)?;
    let mut s = String::new();
    for l in &file.header {
        s.push_str(l);
        s.push('\n');
    }
    for l in hunk {
        s.push_str(l);
        s.push('\n');
    }
    Some(s)
}

/// Apply `patch` with `git apply <args>`, feeding it on stdin. Atomic: a patch
/// that does not apply cleanly leaves the index and working tree unchanged.
fn git_apply(repo_path: &Path, patch: &str, args: &[&str]) -> Result<()> {
    let mut child = Command::new("git")
        .current_dir(repo_path)
        .arg("apply")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| DirigentError::GitCommand("git apply: no stdin".into()))?;
        stdin.write_all(patch.as_bytes())?;
        // stdin dropped here, closing the pipe so git can proceed.
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DirigentError::GitCommand(format!(
            "git apply failed: {}",
            stderr.trim()
        )));
    }
    Ok(())
}

/// Stage a single hunk (working tree -> index): `git apply --cached`.
pub(crate) fn stage_hunk(repo_path: &Path, patch: &str) -> Result<()> {
    git_apply(repo_path, patch, &["--cached"])
}

/// Unstage a single staged hunk (index -> working tree): `git apply --cached --reverse`.
pub(crate) fn unstage_hunk(repo_path: &Path, patch: &str) -> Result<()> {
    git_apply(repo_path, patch, &["--cached", "--reverse"])
}

/// Discard a single unstaged hunk from disk: `git apply --reverse` (no `--cached`).
pub(crate) fn discard_hunk(repo_path: &Path, patch: &str) -> Result<()> {
    git_apply(repo_path, patch, &["--reverse"])
}

/// The staged diff (index vs HEAD): `git diff --cached`. None when nothing staged.
pub(crate) fn get_staged_diff(repo_path: &Path, files: &[String]) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.arg("diff").arg("--cached").current_dir(repo_path);
    if !files.is_empty() {
        cmd.arg("--");
        for f in files {
            cmd.arg(f);
        }
    }
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).to_string();
    if s.trim().is_empty() {
        None
    } else {
        Some(s)
    }
}

/// A file is partially staged when it has both staged (index vs HEAD) and
/// unstaged (working tree vs index) changes.
pub(crate) fn is_partially_staged(repo_path: &Path, file: &str) -> bool {
    let nonempty = |args: &[&str]| -> bool {
        Command::new("git")
            .current_dir(repo_path)
            .args(args)
            .arg("--")
            .arg(file)
            .output()
            .map(|o| o.status.success() && !o.stdout.is_empty())
            .unwrap_or(false)
    };
    nonempty(&["diff", "--cached"]) && nonempty(&["diff"])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn run(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .current_dir(dir)
            .args(args)
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn init_repo(dir: &Path) {
        run(dir, &["init", "-q"]);
        run(dir, &["config", "user.email", "t@t.t"]);
        run(dir, &["config", "user.name", "T"]);
        run(dir, &["config", "commit.gpgsign", "false"]);
    }

    fn working_diff(dir: &Path) -> String {
        String::from_utf8_lossy(
            &Command::new("git")
                .current_dir(dir)
                .args(["diff"])
                .output()
                .unwrap()
                .stdout,
        )
        .into_owned()
    }

    #[test]
    fn split_and_build_single_hunk_patch() {
        let diff = "diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -1,2 +1,2 @@\n-old1\n+new1\n ctx\n@@ -10,1 +10,2 @@\n ctx2\n+added\n";
        let files = split_into_file_diffs(diff);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "f.txt");
        assert_eq!(files[0].hunk_count(), 2);
        let p0 = build_hunk_patch(&files[0], 0).unwrap();
        assert!(p0.contains("@@ -1,2 +1,2 @@"));
        assert!(p0.contains("+new1"));
        // Only the first hunk's lines are present, not the second.
        assert!(!p0.contains("+added"));
        let p1 = build_hunk_patch(&files[0], 1).unwrap();
        assert!(p1.contains("+added") && !p1.contains("+new1"));
        assert!(build_hunk_patch(&files[0], 2).is_none());
    }

    #[test]
    fn binary_file_has_no_hunk_patch() {
        let diff = "diff --git a/img.png b/img.png\nindex 1..2 100644\nBinary files a/img.png and b/img.png differ\n";
        let files = split_into_file_diffs(diff);
        assert_eq!(files.len(), 1);
        assert!(files[0].binary);
        assert_eq!(files[0].hunk_count(), 0);
        assert!(build_hunk_patch(&files[0], 0).is_none());
    }

    #[test]
    fn no_newline_at_eof_marker_preserved() {
        let diff = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n-a\n+b\n\\ No newline at end of file\n";
        let files = split_into_file_diffs(diff);
        let patch = build_hunk_patch(&files[0], 0).unwrap();
        assert!(patch.contains("\\ No newline at end of file"));
    }

    #[test]
    fn stage_one_hunk_leaves_other_unstaged() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        init_repo(p);
        // 20-line file committed, then edit line 1 and line 20 (two hunks apart).
        let base: String = (1..=20).map(|i| format!("line{i}\n")).collect();
        std::fs::write(p.join("f.txt"), &base).unwrap();
        run(p, &["add", "f.txt"]);
        run(p, &["commit", "-qm", "base"]);
        let mut lines: Vec<String> = (1..=20).map(|i| format!("line{i}")).collect();
        lines[0] = "CHANGED1".into();
        lines[19] = "CHANGED20".into();
        std::fs::write(p.join("f.txt"), lines.join("\n") + "\n").unwrap();

        let files = split_into_file_diffs(&working_diff(p));
        assert_eq!(files[0].hunk_count(), 2, "expected two separate hunks");
        let patch = build_hunk_patch(&files[0], 0).unwrap();
        stage_hunk(p, &patch).expect("stage first hunk");

        // The file is now partially staged: staged has hunk 1, working still has hunk 2.
        assert!(is_partially_staged(p, "f.txt"));
        let staged = get_staged_diff(p, &[]).unwrap();
        assert!(staged.contains("CHANGED1"));
        assert!(!staged.contains("CHANGED20"), "second hunk must stay unstaged");
        // Working tree on disk is unchanged (both edits still present).
        let disk = std::fs::read_to_string(p.join("f.txt")).unwrap();
        assert!(disk.contains("CHANGED1") && disk.contains("CHANGED20"));
    }

    #[test]
    fn unstage_hunk_returns_to_working_tree() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        init_repo(p);
        std::fs::write(p.join("f.txt"), "a\nb\nc\n").unwrap();
        run(p, &["add", "f.txt"]);
        run(p, &["commit", "-qm", "base"]);
        std::fs::write(p.join("f.txt"), "A\nb\nc\n").unwrap();
        run(p, &["add", "f.txt"]); // fully staged

        let staged = get_staged_diff(p, &[]).unwrap();
        let files = split_into_file_diffs(&staged);
        let patch = build_hunk_patch(&files[0], 0).unwrap();
        unstage_hunk(p, &patch).expect("unstage");
        // Nothing staged now; the change is back in the working tree.
        assert!(get_staged_diff(p, &[]).is_none());
        assert!(working_diff(p).contains("+A"));
    }

    #[test]
    fn discard_hunk_removes_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        init_repo(p);
        std::fs::write(p.join("f.txt"), "a\nb\nc\n").unwrap();
        run(p, &["add", "f.txt"]);
        run(p, &["commit", "-qm", "base"]);
        std::fs::write(p.join("f.txt"), "A\nb\nc\n").unwrap();

        let files = split_into_file_diffs(&working_diff(p));
        let patch = build_hunk_patch(&files[0], 0).unwrap();
        discard_hunk(p, &patch).expect("discard");
        // The edit is gone from disk; file matches the committed content.
        assert_eq!(std::fs::read_to_string(p.join("f.txt")).unwrap(), "a\nb\nc\n");
    }

    #[test]
    fn rejected_patch_leaves_tree_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        init_repo(p);
        std::fs::write(p.join("f.txt"), "a\nb\nc\n").unwrap();
        run(p, &["add", "f.txt"]);
        run(p, &["commit", "-qm", "base"]);
        // A patch whose context doesn't match the file at all.
        let bogus = "diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -1,1 +1,1 @@\n-zzz\n+yyy\n";
        let err = stage_hunk(p, bogus);
        assert!(err.is_err(), "bogus patch must be rejected");
        assert!(get_staged_diff(p, &[]).is_none(), "index unchanged");
        assert_eq!(std::fs::read_to_string(p.join("f.txt")).unwrap(), "a\nb\nc\n");
    }
}
