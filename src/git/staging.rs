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

use std::collections::HashSet;
use std::io::Write;
use std::path::Path;
use std::process::Stdio;

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

    /// The `@@ ... @@` header line of hunk `idx`.
    pub fn hunk_header(&self, idx: usize) -> Option<&str> {
        self.hunks
            .get(idx)
            .and_then(|h| h.first())
            .map(|s| s.as_str())
    }

    /// The verbatim lines of hunk `idx` (starting with its `@@` header), for
    /// presenting numbered hunks to the grouping model.
    pub fn hunk_lines(&self, idx: usize) -> Option<&[String]> {
        self.hunks.get(idx).map(|h| h.as_slice())
    }
}

/// Parse the new-side range `+c,d` from an `@@ -a,b +c,d @@` header into
/// `(start, len)`. A missing length defaults to 1.
fn parse_new_range(header: &str) -> Option<(usize, usize)> {
    let plus = header.find('+')?;
    let rest = &header[plus + 1..];
    let end = rest.find(' ').unwrap_or(rest.len());
    let spec = &rest[..end];
    let mut parts = spec.split(',');
    let start: usize = parts.next()?.trim().parse().ok()?;
    let len: usize = match parts.next() {
        Some(l) => l.trim().parse().ok()?,
        None => 1,
    };
    Some((start, len))
}

/// Match a stored hunk header against a file's current hunks: exact header first,
/// then unambiguous new-side range overlap. Returns the real hunk index, or
/// `None` when there is no match or the overlap is ambiguous.
pub(crate) fn match_hunk_header(file: &FileRawDiff, header: &str) -> Option<usize> {
    // 1. Exact header match.
    if let Some(idx) = (0..file.hunks.len()).find(|&i| file.hunk_header(i) == Some(header)) {
        return Some(idx);
    }
    // 2. New-side range overlap, but only if exactly one hunk overlaps. Both
    //    ranges are widened to at least one line so a zero-length range (a pure
    //    deletion like `+9,0`) can still overlap an equal or shifted match.
    let (ws, wl) = parse_new_range(header)?;
    let want_end = ws + wl.max(1);
    let mut matches = (0..file.hunks.len()).filter(|&i| {
        file.hunk_header(i)
            .and_then(parse_new_range)
            .map(|(s, l)| s < want_end && ws < s + l.max(1))
            .unwrap_or(false)
    });
    let first = matches.next()?;
    if matches.next().is_some() {
        None // ambiguous: more than one hunk overlaps
    } else {
        Some(first)
    }
}

/// Split a raw (possibly multi-file) unified diff into per-file sections,
/// preserving every line verbatim.
pub(crate) fn split_into_file_diffs(diff: &str) -> Vec<FileRawDiff> {
    let mut builder = RawDiffBuilder::default();
    for line in diff.lines() {
        builder.push_line(line);
    }
    builder.finish()
}

#[derive(Default)]
struct RawDiffBuilder {
    files: Vec<FileRawDiff>,
    cur: Option<FileRawDiff>,
    in_hunks: bool,
}

impl RawDiffBuilder {
    fn push_line(&mut self, line: &str) {
        if line.starts_with("diff --git ") {
            self.start_file(line);
        } else if line.starts_with("@@ ") {
            self.start_hunk(line);
        } else if self.in_hunks {
            self.push_hunk_line(line);
        } else {
            self.push_header_line(line);
        }
    }

    fn start_file(&mut self, line: &str) {
        self.flush_current();
        self.cur = Some(FileRawDiff {
            path: parse_diff_git_path(line),
            header: vec![line.to_string()],
            hunks: Vec::new(),
            binary: false,
        });
        self.in_hunks = false;
    }

    fn start_hunk(&mut self, line: &str) {
        if let Some(file) = self.cur.as_mut() {
            self.in_hunks = true;
            file.hunks.push(vec![line.to_string()]);
        }
    }

    fn push_hunk_line(&mut self, line: &str) {
        if let Some(hunk) = self.cur.as_mut().and_then(|file| file.hunks.last_mut()) {
            hunk.push(line.to_string());
        }
    }

    fn push_header_line(&mut self, line: &str) {
        let Some(file) = self.cur.as_mut() else {
            return;
        };
        update_file_header_from_line(file, line);
        file.header.push(line.to_string());
    }

    fn flush_current(&mut self) {
        if let Some(file) = self.cur.take() {
            self.files.push(file);
        }
    }

    fn finish(mut self) -> Vec<FileRawDiff> {
        self.flush_current();
        self.files
    }
}

fn update_file_header_from_line(file: &mut FileRawDiff, line: &str) {
    if line.starts_with("Binary files ") || line.starts_with("GIT binary patch") {
        file.binary = true;
    }
    if let Some(path) = plus_header_path(line) {
        file.path = path;
    } else if let Some(path) = minus_header_path(line, file.path.is_empty()) {
        file.path = path;
    }
}

fn plus_header_path(line: &str) -> Option<String> {
    line.strip_prefix("+++ ")
        .filter(|path| *path != "/dev/null")
        .map(strip_ab_prefix)
}

fn minus_header_path(line: &str, path_empty: bool) -> Option<String> {
    line.strip_prefix("--- ")
        .filter(|path| path_empty && *path != "/dev/null")
        .map(strip_ab_prefix)
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
    let mut child = super::git_cmd()
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

/// Take an undo point for a destructive working-tree operation.
///
/// `git stash create` builds a stash *commit* from the current index and
/// working tree and prints its hash, without touching either — so it is a free
/// snapshot rather than a state change. The commit is initially unreferenced,
/// so it is anchored with `git stash store` to keep it out of reach of `gc`.
///
/// Returns `None` when there is nothing to snapshot (a clean tree) or git
/// failed; discarding still proceeds in that case, just without an undo point.
pub(crate) fn snapshot_for_undo(repo_path: &Path, label: &str) -> Option<String> {
    let out = super::git_cmd()
        .args(["stash", "create", label])
        .current_dir(repo_path)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let hash = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if hash.is_empty() {
        return None;
    }
    // Anchor the dangling commit in the stash reflog so `git gc` cannot
    // collect the user's only copy of what they are about to discard.
    let _ = super::git_cmd()
        .args(["stash", "store", "-m", label, &hash])
        .current_dir(repo_path)
        .output();
    Some(hash)
}

/// The staged diff (index vs HEAD): `git diff --cached`. None when nothing staged.
pub(crate) fn get_staged_diff(repo_path: &Path, files: &[String]) -> Option<String> {
    let mut cmd = super::git_cmd();
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

/// Every path that is partially staged — it has both staged (index vs HEAD)
/// and unstaged (working tree vs index) changes.
///
/// One `git status --porcelain=v2 -z` call answers this for the whole
/// repository. The previous per-file `is_partially_staged` spawned two `git
/// diff` processes *per file*, synchronously on the UI thread, on every staging
/// refresh — around 100 process spawns for a 50-file diff, once per click.
pub(crate) fn partially_staged_files(repo_path: &Path) -> HashSet<String> {
    let output = super::git_cmd()
        .current_dir(repo_path)
        .args(["status", "--porcelain=v2", "-z"])
        .output();
    let Ok(output) = output else {
        return HashSet::new();
    };
    if !output.status.success() {
        return HashSet::new();
    }
    parse_partially_staged(&String::from_utf8_lossy(&output.stdout))
}

/// Parse `git status --porcelain=v2 -z` output into the set of partially
/// staged paths.
///
/// Records are NUL-separated. The two that carry an XY status pair are:
///
/// ```text
/// 1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>
/// 2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <X><score> <path>\0<origPath>
/// ```
///
/// `X` is the index-vs-HEAD status and `Y` the worktree-vs-index status; `.`
/// means unchanged on that side. Both non-`.` is exactly "partially staged".
/// A rename record is followed by an extra NUL-terminated original path, which
/// is skipped so it is not mistaken for the next record.
fn parse_partially_staged(stdout: &str) -> HashSet<String> {
    let mut result = HashSet::new();
    let mut records = stdout.split('\0');
    while let Some(record) = records.next() {
        if record.is_empty() {
            continue;
        }
        let mut fields = record.splitn(2, ' ');
        let kind = fields.next().unwrap_or_default();
        // Rename/copy records carry their original path as a separate record.
        let path_field_index = match kind {
            "1" => 8,
            "2" => {
                records.next();
                9
            }
            // "u" (unmerged), "?" (untracked) and "!" (ignored) are never
            // "partially staged" in the index-vs-worktree sense.
            _ => continue,
        };
        let parts: Vec<&str> = record.splitn(path_field_index + 1, ' ').collect();
        if parts.len() <= path_field_index {
            continue;
        }
        let xy = parts[1].as_bytes();
        if xy.len() == 2 && xy[0] != b'.' && xy[1] != b'.' {
            result.insert(parts[path_field_index].to_string());
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(dir: &Path, args: &[&str]) -> String {
        let out = crate::git::git_cmd()
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
            &crate::git::git_cmd()
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
        assert_eq!(files[0].hunks.len(), 2);
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
        assert_eq!(files[0].hunks.len(), 0);
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
        assert_eq!(files[0].hunks.len(), 2, "expected two separate hunks");
        let patch = build_hunk_patch(&files[0], 0).unwrap();
        stage_hunk(p, &patch).expect("stage first hunk");

        // The file is now partially staged: staged has hunk 1, working still has hunk 2.
        assert!(partially_staged_files(p).contains("f.txt"));
        let staged = get_staged_diff(p, &[]).unwrap();
        assert!(staged.contains("CHANGED1"));
        assert!(
            !staged.contains("CHANGED20"),
            "second hunk must stay unstaged"
        );
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
        assert_eq!(
            std::fs::read_to_string(p.join("f.txt")).unwrap(),
            "a\nb\nc\n"
        );
    }

    #[test]
    fn match_hunk_header_exact_and_overlap() {
        let diff = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1,2 +1,3 @@\n ctx\n+add\n@@ -20,1 +21,1 @@\n-x\n+y\n";
        let files = split_into_file_diffs(diff);
        let f = &files[0];
        // Exact header match.
        assert_eq!(match_hunk_header(f, "@@ -1,2 +1,3 @@"), Some(0));
        assert_eq!(match_hunk_header(f, "@@ -20,1 +21,1 @@"), Some(1));
        // Overlap match: a header with drifted old-side but overlapping new range.
        assert_eq!(match_hunk_header(f, "@@ -1,2 +1,2 @@"), Some(0));
        // No overlap at all.
        assert_eq!(match_hunk_header(f, "@@ -99,1 +99,1 @@"), None);
    }

    #[test]
    fn match_hunk_header_zero_length_deletion_range() {
        // A pure deletion has a zero-length new-side range (`+9,0`).
        let diff = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -10,3 +9,0 @@\n-x\n-y\n-z\n";
        let files = split_into_file_diffs(diff);
        let f = &files[0];
        // Old side drifted (earlier commit shifted lines), equal new-side range:
        // must still match despite both ranges being zero-length.
        assert_eq!(match_hunk_header(f, "@@ -12,3 +9,0 @@"), Some(0));
        // Overlap between a zero-length wanted range and a non-empty hunk range.
        let diff2 = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -8,4 +8,2 @@\n ctx\n-x\n-y\n ctx2\n";
        let files2 = split_into_file_diffs(diff2);
        assert_eq!(match_hunk_header(&files2[0], "@@ -10,2 +9,0 @@"), Some(0));
    }

    #[test]
    fn commit_partial_commits_only_selected_hunks() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        init_repo(p);
        let base: String = (1..=30).map(|i| format!("line{i}\n")).collect();
        std::fs::write(p.join("f.txt"), &base).unwrap();
        run(p, &["add", "f.txt"]);
        run(p, &["commit", "-qm", "base"]);
        // Two far-apart edits -> two hunks.
        let mut lines: Vec<String> = (1..=30).map(|i| format!("line{i}")).collect();
        lines[0] = "CHANGED1".into();
        lines[29] = "CHANGED30".into();
        std::fs::write(p.join("f.txt"), lines.join("\n") + "\n").unwrap();

        let files = split_into_file_diffs(&working_diff(p));
        let patch0 = build_hunk_patch(&files[0], 0).unwrap();
        // Commit only the first hunk.
        crate::git::commit_partial(p, &[], &[patch0], "part: first hunk").expect("partial commit");

        // HEAD now contains CHANGED1 but not CHANGED30.
        let head = run(p, &["show", "HEAD:f.txt"]);
        assert!(head.contains("CHANGED1"));
        assert!(!head.contains("CHANGED30"));
        // The second hunk is still dirty in the working tree.
        let wd = working_diff(p);
        assert!(wd.contains("CHANGED30"));
        assert!(
            !wd.contains("CHANGED1"),
            "committed hunk should no longer be dirty"
        );
    }

    #[test]
    fn failed_partial_commit_rolls_back_staging() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        init_repo(p);
        std::fs::write(p.join("a.txt"), "a\n").unwrap();
        std::fs::write(p.join("b.txt"), "b\n").unwrap();
        run(p, &["add", "."]);
        run(p, &["commit", "-qm", "base"]);
        std::fs::write(p.join("a.txt"), "A\n").unwrap();

        // Whole file `a.txt` stages fine, then the bogus hunk is rejected.
        let bogus =
            "diff --git a/b.txt b/b.txt\n--- a/b.txt\n+++ b/b.txt\n@@ -1,1 +1,1 @@\n-zzz\n+yyy\n";
        let err = crate::git::commit_partial(
            p,
            &["a.txt".to_string()],
            &[bogus.to_string()],
            "part: must fail",
        );
        assert!(err.is_err(), "rejected hunk must fail the commit");
        // The index was rolled back: nothing is left staged from `a.txt`.
        assert!(
            get_staged_diff(p, &[]).is_none(),
            "failed partial commit must not leave the index staged"
        );
        // The working tree still has the edit.
        assert_eq!(std::fs::read_to_string(p.join("a.txt")).unwrap(), "A\n");
    }

    #[test]
    fn commit_partial_preserves_unrelated_staged_entry() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        init_repo(p);
        std::fs::write(p.join("a.txt"), "a\n").unwrap();
        std::fs::write(p.join("b.txt"), "b\n").unwrap();
        run(p, &["add", "."]);
        run(p, &["commit", "-qm", "base"]);

        // Stage a change to b.txt through the hunk UI path, then regenerate
        // the file on disk without it — the staged blob is the only copy left.
        std::fs::write(p.join("b.txt"), "B\n").unwrap();
        let files = split_into_file_diffs(&working_diff(p));
        let b_diff = files.iter().find(|f| f.path.ends_with("b.txt")).unwrap();
        let patch = build_hunk_patch(b_diff, 0).unwrap();
        stage_hunk(p, &patch).expect("stage b.txt hunk");
        std::fs::write(p.join("b.txt"), "regenerated\n").unwrap();

        // Commit an unrelated change to a.txt.
        std::fs::write(p.join("a.txt"), "A\n").unwrap();
        crate::git::commit_partial(p, &["a.txt".to_string()], &[], "fix: a only")
            .expect("commit a.txt");

        // The commit contains only a.txt; b.txt's staged content survived.
        assert!(run(p, &["show", "HEAD:a.txt"]).contains('A'));
        assert_eq!(run(p, &["show", "HEAD:b.txt"]), "b\n");
        let staged = get_staged_diff(p, &[]).expect("b.txt must still be staged");
        assert!(staged.contains("b.txt") && staged.contains("+B"));
        assert!(!staged.contains("a.txt"), "a.txt must not stay staged");
    }

    #[test]
    fn commit_diff_preserves_unrelated_staged_entry() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        init_repo(p);
        std::fs::write(p.join("a.txt"), "a\n").unwrap();
        std::fs::write(p.join("b.txt"), "b\n").unwrap();
        run(p, &["add", "."]);
        run(p, &["commit", "-qm", "base"]);

        // Externally staged change to b.txt, then the file changes on disk.
        std::fs::write(p.join("b.txt"), "B\n").unwrap();
        run(p, &["add", "b.txt"]);
        std::fs::write(p.join("b.txt"), "regenerated\n").unwrap();

        std::fs::write(p.join("a.txt"), "A\n").unwrap();
        let diff =
            "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1,1 +1,1 @@\n-a\n+A\n";
        crate::git::commit_diff(p, diff, "fix: a only").expect("commit diff");

        assert!(run(p, &["show", "HEAD:a.txt"]).contains('A'));
        assert_eq!(run(p, &["show", "HEAD:b.txt"]), "b\n");
        let staged = get_staged_diff(p, &[]).expect("b.txt must still be staged");
        assert!(staged.contains("b.txt") && staged.contains("+B"));
    }

    #[test]
    fn failed_partial_commit_restores_prestaged_entries() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        init_repo(p);
        std::fs::write(p.join("a.txt"), "a\n").unwrap();
        std::fs::write(p.join("b.txt"), "b\n").unwrap();
        run(p, &["add", "."]);
        run(p, &["commit", "-qm", "base"]);

        std::fs::write(p.join("b.txt"), "B\n").unwrap();
        run(p, &["add", "b.txt"]);
        std::fs::write(p.join("a.txt"), "A\n").unwrap();

        // The bogus hunk (nonexistent file) makes the commit fail after a.txt
        // was staged.
        let bogus =
            "diff --git a/c.txt b/c.txt\n--- a/c.txt\n+++ b/c.txt\n@@ -1,1 +1,1 @@\n-zzz\n+yyy\n";
        let err = crate::git::commit_partial(
            p,
            &["a.txt".to_string()],
            &[bogus.to_string()],
            "part: must fail",
        );
        assert!(err.is_err(), "rejected hunk must fail the commit");

        // Rollback removed a.txt from the index but kept b.txt's staged state.
        let staged = get_staged_diff(p, &[]).expect("b.txt must still be staged");
        assert!(staged.contains("b.txt") && staged.contains("+B"));
        assert!(
            !staged.contains("a.txt"),
            "a.txt staging must be rolled back"
        );
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
        let bogus =
            "diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -1,1 +1,1 @@\n-zzz\n+yyy\n";
        let err = stage_hunk(p, bogus);
        assert!(err.is_err(), "bogus patch must be rejected");
        assert!(get_staged_diff(p, &[]).is_none(), "index unchanged");
        assert_eq!(
            std::fs::read_to_string(p.join("f.txt")).unwrap(),
            "a\nb\nc\n"
        );
    }

    // -- porcelain=v2 partial-staging parsing --

    #[test]
    fn parse_partially_staged_picks_only_both_sides_modified() {
        // `1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>`, NUL-separated.
        let stdout = concat!(
            "1 MM N... 100644 100644 100644 aaa bbb both.rs\0",
            "1 M. N... 100644 100644 100644 aaa bbb staged_only.rs\0",
            "1 .M N... 100644 100644 100644 aaa bbb worktree_only.rs\0",
            "? untracked.rs\0",
        );
        let set = parse_partially_staged(stdout);
        assert_eq!(set.len(), 1, "got: {set:?}");
        assert!(set.contains("both.rs"));
    }

    /// Rename records carry their original path as an extra NUL-terminated
    /// field; skipping it wrong would misparse every following record.
    #[test]
    fn parse_partially_staged_handles_rename_records() {
        let stdout = concat!(
            "2 RM N... 100644 100644 100644 aaa bbb R100 new_name.rs\0old_name.rs\0",
            "1 MM N... 100644 100644 100644 aaa bbb after.rs\0",
        );
        let set = parse_partially_staged(stdout);
        assert!(set.contains("new_name.rs"), "got: {set:?}");
        assert!(
            set.contains("after.rs"),
            "the record after a rename must still parse: {set:?}"
        );
        assert!(!set.contains("old_name.rs"));
    }

    #[test]
    fn parse_partially_staged_on_clean_repo_is_empty() {
        assert!(parse_partially_staged("").is_empty());
    }

    /// Paths with spaces survive: only the fields *before* the path are split.
    #[test]
    fn parse_partially_staged_keeps_spaces_in_paths() {
        let stdout = "1 MM N... 100644 100644 100644 aaa bbb dir with spaces/f.rs\0";
        let set = parse_partially_staged(stdout);
        assert!(set.contains("dir with spaces/f.rs"), "got: {set:?}");
    }
}
