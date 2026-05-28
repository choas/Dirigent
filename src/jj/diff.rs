use std::path::Path;

/// Get the working-tree diff via `jj diff` in git-compatible format.
pub(crate) fn jj_get_working_diff(
    repo_path: &Path,
    files: &[String],
    jj_path: &str,
) -> Option<String> {
    let mut cmd = super::jj_cmd(jj_path);
    cmd.args(["diff", "--git", "--color", "never"])
        .current_dir(repo_path);
    if !files.is_empty() {
        cmd.arg("--");
        for f in files {
            cmd.arg(f);
        }
    }

    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }

    let diff = String::from_utf8_lossy(&output.stdout).to_string();
    if diff.trim().is_empty() {
        None
    } else {
        Some(diff)
    }
}
