use std::path::Path;

use crate::git::CommitInfo;

/// Read commit history via `jj log`.
///
/// Uses change_id as the primary identifier (stored in `full_hash` and
/// `parent_hashes`) so that graph computation and diff lookups work with
/// jj's native revision identifiers.
pub(crate) fn jj_read_commit_history(path: &Path, limit: usize, jj_path: &str) -> Vec<CommitInfo> {
    let limit_str = limit.to_string();
    let output = super::jj_cmd(jj_path)
        .args([
            "log",
            "--no-graph",
            "-n",
            &limit_str,
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
        ])
        .current_dir(path)
        .output();

    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut result = Vec::new();
    let mut is_first = true;

    for line in stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 8 {
            continue;
        }

        let change_id = parts[0].trim().to_string();
        let short_hash = if change_id.len() >= 7 {
            change_id[..7].to_string()
        } else {
            change_id.clone()
        };
        // Pull trailing fields from the end so tabs in the description don't shift offsets.
        let n = parts.len();
        let is_empty = parts[n - 1].trim() == "empty";
        let parents_str = parts[n - 2].trim();
        let tags_str = parts[n - 3].trim();
        let bookmarks_str = parts[n - 4].trim();
        let timestamp_str = parts[n - 5].trim();
        let author = parts[n - 6].trim().to_string();
        let mut message = parts[1..n - 6].join("\t").trim().to_string();

        if message.is_empty() {
            message = "(no description yet)".to_string();
        }

        let wc_prefix = if is_first { "@ " } else { "" };
        let empty_suffix = if is_empty { " (empty)" } else { "" };
        message = format!("{}{}{}", wc_prefix, message, empty_suffix);

        let time_ago = parse_jj_timestamp(timestamp_str, now);

        let branch_labels: Vec<String> = if bookmarks_str.is_empty() {
            Vec::new()
        } else {
            bookmarks_str
                .split_whitespace()
                .map(|s| s.to_string())
                .collect()
        };

        let tag_labels: Vec<String> = if tags_str.is_empty() {
            Vec::new()
        } else {
            tags_str.split_whitespace().map(|s| s.to_string()).collect()
        };

        let parent_hashes: Vec<String> = if parents_str.is_empty() {
            Vec::new()
        } else {
            parents_str
                .split(',')
                .map(|s| s.trim().to_string())
                .collect()
        };

        let is_merge = parent_hashes.len() > 1;

        result.push(CommitInfo {
            full_hash: change_id,
            short_hash,
            message,
            body: String::new(),
            author,
            time_ago,
            parent_hashes,
            branch_labels,
            tag_labels,
            is_merge,
            is_working_copy: is_first,
        });
        is_first = false;
    }

    result
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
    let output = super::jj_cmd(jj_path)
        .args(["log", "--no-graph", "-T", r#"change_id ++ "\n""#])
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

pub(crate) fn jj_get_commit_diff(path: &Path, change_id: &str, jj_path: &str) -> Option<String> {
    let output = super::jj_cmd(jj_path)
        .args(["diff", "--git", "-r", change_id])
        .current_dir(path)
        .output()
        .ok()?;

    if output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout).to_string();
        if text.trim().is_empty() {
            None
        } else {
            Some(text)
        }
    } else {
        None
    }
}
