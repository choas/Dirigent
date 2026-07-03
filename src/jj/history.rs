use std::path::Path;

use crate::git::CommitInfo;

/// Read commit history via `jj log`.
///
/// Uses change_id as the primary identifier (stored in `full_hash` and
/// `parent_hashes`) so that graph computation and diff lookups work with
/// jj's native revision identifiers.
pub(crate) fn jj_read_commit_history(path: &Path, limit: usize, jj_path: &str) -> Vec<CommitInfo> {
    let limit_str = limit.to_string();
    let Some(stdout) = jj_log_output(path, jj_path, &limit_str) else {
        return Vec::new();
    };
    parse_jj_log_output(&stdout)
}

fn jj_log_output(path: &Path, jj_path: &str, limit: &str) -> Option<String> {
    jj_stdout(
        path,
        jj_path,
        &[
            "log",
            "--no-graph",
            "--color",
            "never",
            "-n",
            limit,
            "-T",
            concat!(
                r#"change_id ++ "\t""#,
                r#" ++ description.first_line() ++ "\t""#,
                r#" ++ author.name() ++ "\t""#,
                r#" ++ author.timestamp() ++ "\t""#,
                r#" ++ bookmarks ++ "\t""#,
                r#" ++ tags ++ "\t""#,
                r#" ++ parents.map(|p| p.change_id()).join(",") ++ "\t""#,
                r#" ++ if(empty, "empty", "") ++ "\n""#,
            ),
        ],
    )
}

fn jj_stdout(path: &Path, jj_path: &str, args: &[&str]) -> Option<String> {
    let output = super::jj_cmd(jj_path)
        .args(args)
        .current_dir(path)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).to_string())
}

fn parse_jj_log_output(stdout: &str) -> Vec<CommitInfo> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let mut result = Vec::new();
    let mut is_first = true;

    for line in stdout.lines() {
        let Some(commit) = parse_jj_log_line(line, now, is_first) else {
            continue;
        };
        result.push(commit);
        is_first = false;
    }

    result
}

fn parse_jj_log_line(line: &str, now: i64, is_first: bool) -> Option<CommitInfo> {
    if line.trim().is_empty() {
        return None;
    }
    let parts: Vec<&str> = line.split('\t').collect();
    if parts.len() < 8 {
        return None;
    }

    let n = parts.len();
    let change_id = parts[0].trim().to_string();
    let parent_hashes = comma_labels(parts[n - 2].trim());
    Some(CommitInfo {
        short_hash: short_change_id(&change_id),
        message: jj_commit_message(&parts, is_first),
        author: parts[n - 6].trim().to_string(),
        time_ago: parse_jj_timestamp(parts[n - 5].trim(), now),
        branch_labels: whitespace_labels(parts[n - 4].trim()),
        tag_labels: whitespace_labels(parts[n - 3].trim()),
        is_merge: parent_hashes.len() > 1,
        parent_hashes,
        full_hash: change_id,
        body: String::new(),
        is_working_copy: is_first,
    })
}

fn short_change_id(change_id: &str) -> String {
    if change_id.len() >= 7 {
        change_id[..7].to_string()
    } else {
        change_id.to_string()
    }
}

fn jj_commit_message(parts: &[&str], is_first: bool) -> String {
    let n = parts.len();
    let mut message = parts[1..n - 6].join("\t").trim().to_string();
    if message.is_empty() {
        message = "(no description yet)".to_string();
    }
    let wc_prefix = if is_first { "@ " } else { "" };
    let empty_suffix = if parts[n - 1].trim() == "empty" {
        " (empty)"
    } else {
        ""
    };
    format!("{}{}{}", wc_prefix, message, empty_suffix)
}

fn whitespace_labels(labels: &str) -> Vec<String> {
    labels.split_whitespace().map(|s| s.to_string()).collect()
}

fn comma_labels(labels: &str) -> Vec<String> {
    labels
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

fn parse_jj_timestamp(ts: &str, now: i64) -> String {
    // jj timestamps are RFC 3339 style; do a rough parse
    if let Some(secs) = parse_rfc3339_rough(ts) {
        let diff = now - secs;
        format_time_ago(diff)
    } else {
        ts.to_string()
    }
}

fn parse_rfc3339_rough(ts: &str) -> Option<i64> {
    // Very rough parsing: "2025-01-15 12:34:56.000 -05:00" or ISO format
    let ts = ts.trim();
    // Try chrono-free parsing: extract date+time parts
    let date_part = ts.get(..10)?;
    let time_part = ts.get(11..19).unwrap_or("00:00:00");

    let mut parts = date_part.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;

    let mut tparts = time_part.split(':');
    let hour: i64 = tparts.next()?.parse().ok()?;
    let min: i64 = tparts.next()?.parse().ok()?;
    let sec: i64 = tparts.next()?.parse().ok()?;

    // Rough epoch calculation (ignoring leap years/seconds for time-ago display)
    let days = (year - 1970) * 365 + (year - 1969) / 4 + month_days(month) + day - 1;
    Some(days * 86400 + hour * 3600 + min * 60 + sec)
}

fn month_days(month: i64) -> i64 {
    const CUMULATIVE: [i64; 13] = [0, 0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    CUMULATIVE.get(month as usize).copied().unwrap_or(0)
}

fn format_time_ago(diff: i64) -> String {
    if diff <= 0 || diff < 60 {
        "just now".to_string()
    } else if diff < 3600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86400 {
        format!("{}h ago", diff / 3600)
    } else {
        format!("{}d ago", diff / 86400)
    }
}

pub(crate) fn jj_count_commits(path: &Path, jj_path: &str) -> usize {
    jj_stdout(
        path,
        jj_path,
        &[
            "log",
            "--no-graph",
            "--color",
            "never",
            "--ignore-working-copy",
            "-T",
            r#"change_id ++ "\n""#,
        ],
    )
    .map(|stdout| stdout.lines().filter(|l| !l.trim().is_empty()).count())
    .unwrap_or(0)
}

/// Find empty head commits (leaves of the commit graph that have no diff).
///
/// Uses `heads(all())` to find all head revisions, then filters to those
/// that are marked `empty` by jj.  Returns a vec of `(change_id, description)`.
pub(crate) fn jj_find_empty_heads(path: &Path, jj_path: &str) -> Vec<(String, String)> {
    let Some(stdout) = jj_stdout(
        path,
        jj_path,
        &[
            "log",
            "--no-graph",
            "--color",
            "never",
            "--ignore-working-copy",
            "-r",
            "heads(all()) & empty()",
            "-T",
            r#"change_id ++ "\t" ++ description.first_line() ++ "\n""#,
        ],
    ) else {
        return Vec::new();
    };
    stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|line| {
            let (id, desc) = line.split_once('\t')?;
            Some((id.trim().to_string(), desc.trim().to_string()))
        })
        .collect()
}

pub(crate) fn jj_get_commit_diff(path: &Path, change_id: &str, jj_path: &str) -> Option<String> {
    jj_stdout(
        path,
        jj_path,
        &[
            "diff",
            "--git",
            "--color",
            "never",
            "--ignore-working-copy",
            "-r",
            change_id,
        ],
    )
}
