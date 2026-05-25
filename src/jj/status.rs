use std::collections::HashMap;
use std::path::Path;

use crate::git::GitInfo;

/// Read repository info via `jj log` for the working-copy commit.
pub(crate) fn jj_read_info(path: &Path, jj_path: &str) -> Option<GitInfo> {
    // Get the current bookmark (branch) name, change id, and description
    let output = super::jj_cmd(jj_path)
        .args([
            "log",
            "-r",
            "@",
            "--no-graph",
            "-T",
            r#"bookmarks ++ "\n" ++ change_id.shortest(7) ++ "\n" ++ description.first_line()"#,
        ])
        .current_dir(path)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();

    let mut branch = lines
        .first()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_default();

    let mut change_id = lines
        .get(1)
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    let mut description = lines
        .get(2)
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "(no description)".to_string());

    // If @ has no bookmark, check the parent — after `jj commit` the
    // bookmark lives on @- and the new @ is an empty child without one.
    // Re-read all fields from @- so branch, change_id, and description
    // are consistent.
    if branch.is_empty() {
        let parent_out = super::jj_cmd(jj_path)
            .args([
                "log",
                "-r",
                "@-",
                "--no-graph",
                "-T",
                r#"bookmarks ++ "\n" ++ change_id.shortest(7) ++ "\n" ++ description.first_line()"#,
            ])
            .current_dir(path)
            .output()
            .ok();
        if let Some(po) = parent_out {
            if po.status.success() {
                let parent_stdout = String::from_utf8_lossy(&po.stdout).into_owned();
                let parent_lines: Vec<&str> = parent_stdout.lines().collect();
                let parent_bm = parent_lines
                    .first()
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default();
                if !parent_bm.is_empty() {
                    branch = parent_bm;
                    if let Some(cid) = parent_lines.get(1) {
                        change_id = cid.trim().to_string();
                    }
                    description = parent_lines
                        .get(2)
                        .map(|s| s.trim().to_string())
                        .unwrap_or_else(|| "(no description)".to_string());
                }
            }
        }
    }

    if branch.is_empty() {
        branch = "(no bookmark)".to_string();
    }

    // Count status entries via `jj diff --types`
    let (modified, added, deleted, conflicted) = count_status_entries(path, jj_path);

    Some(GitInfo {
        branch,
        last_commit_hash: change_id,
        last_commit_message: description,
        modified_count: modified,
        added_count: added,
        deleted_count: deleted,
        conflicted_count: conflicted,
    })
}

fn count_status_entries(path: &Path, jj_path: &str) -> (usize, usize, usize, usize) {
    let output = super::jj_cmd(jj_path)
        .args(["diff", "--types"])
        .current_dir(path)
        .output();

    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return (0, 0, 0, 0),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut modified = 0;
    let mut added = 0;
    let mut deleted = 0;
    let mut conflicted = 0;

    for line in stdout.lines() {
        // Format: "FT path" where F=from-type, T=to-type
        // Types: F=file, D=directory, L=symlink, C=conflict, -=absent
        let trimmed = line.trim();
        if trimmed.len() < 3 {
            continue;
        }
        let from_type = trimmed.as_bytes()[0];
        let to_type = trimmed.as_bytes()[1];
        match (from_type, to_type) {
            (b'-', _) => added += 1,
            (_, b'-') => deleted += 1,
            (b'C', _) | (_, b'C') => conflicted += 1,
            _ => modified += 1,
        }
    }
    (modified, added, deleted, conflicted)
}

/// Returns relative paths of all files with uncommitted changes in the working copy.
pub(crate) fn jj_get_dirty_files(path: &Path, jj_path: &str) -> HashMap<String, char> {
    let mut result = HashMap::new();

    let output = super::jj_cmd(jj_path)
        .args(["diff", "--types"])
        .current_dir(path)
        .output();

    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return result,
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        // Format: "FT path" where F=from-type, T=to-type
        // Types: F=file, D=directory, L=symlink, C=conflict, -=absent
        let trimmed = line.trim();
        if trimmed.len() < 3 {
            continue;
        }
        let from_type = trimmed.as_bytes()[0];
        let to_type = trimmed.as_bytes()[1];
        let file_path = trimmed[2..].trim().to_string();

        if file_path.is_empty() {
            continue;
        }

        let letter = match (from_type, to_type) {
            (b'-', _) => '?',             // new file (absent -> something)
            (_, b'-') => 'D',             // deleted (something -> absent)
            (b'C', _) | (_, b'C') => 'U', // conflict
            _ => 'M',                     // modified
        };
        result.insert(file_path, letter);
    }
    result
}

/// Returns the number of commits ahead of the tracked remote bookmark.
pub(crate) fn jj_get_ahead_of_remote(path: &Path, jj_path: &str) -> usize {
    // Count revisions between the current bookmark's remote tracking and local tip
    let output = super::jj_cmd(jj_path)
        .args([
            "log",
            "-r",
            "remote_bookmarks()..@-",
            "--no-graph",
            "-T",
            r#"change_id ++ "\n""#,
        ])
        .current_dir(path)
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            stdout.lines().filter(|l| !l.trim().is_empty()).count()
        }
        _ => 0,
    }
}
