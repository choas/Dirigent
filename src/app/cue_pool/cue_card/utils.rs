use std::time::SystemTime;

use eframe::egui;

use crate::db::Cue;

/// Compute display text for a cue, truncating if it's long and not expanded.
pub(in crate::app) fn compute_display_text(cue: &Cue, is_expanded: bool) -> String {
    let line_count = cue.text.lines().count();
    let word_count = cue.text.split_whitespace().count();
    let is_long = line_count > 10 || word_count > 50;

    if is_long && !is_expanded {
        let truncated: String = cue.text.lines().take(5).collect::<Vec<_>>().join("\n");
        let words: Vec<&str> = truncated.split_whitespace().collect();
        if words.len() > 50 {
            format!("{}\u{2026}", words[..50].join(" "))
        } else {
            format!("{}\u{2026}", truncated)
        }
    } else {
        cue.text.clone()
    }
}

/// Format queue/schedule label for display.
pub(in crate::app) fn format_queue_label(
    is_queued: bool,
    scheduled_when: Option<SystemTime>,
) -> String {
    if is_queued {
        return "\u{23F3} Queued".to_string();
    }
    if let Some(when) = scheduled_when {
        let now = SystemTime::now();
        if when <= now {
            return "\u{23F2} Due now".to_string();
        }
        let secs = when.duration_since(now).unwrap_or_default().as_secs();
        if secs < 60 {
            return format!("\u{23F2} {}s", secs);
        }
        if secs < 3600 {
            return format!("\u{23F2} {}:{:02}", secs / 60, secs % 60);
        }
        return format!("\u{23F2} {}h{}m", secs / 3600, (secs % 3600) / 60);
    }
    "\u{23F3} Pending".to_string()
}

/// Toggle reply input visibility for a cue.
pub(in crate::app) fn toggle_reply_input(
    reply_inputs: &mut std::collections::HashMap<i64, String>,
    cue_id: i64,
) {
    if let std::collections::hash_map::Entry::Vacant(e) = reply_inputs.entry(cue_id) {
        e.insert(String::new());
    } else {
        reply_inputs.remove(&cue_id);
    }
}

/// Detect if an activity event was logged by one of the given agents and
/// return that agent's db key. `agents` maps display name -> db key; longer
/// names are tried first so a custom agent named "Test Coverage" wins over
/// the builtin "Test" label.
pub(in crate::app) fn detect_agent_event(
    event: &str,
    agents: &[(String, String)],
) -> Option<String> {
    let mut candidates: Vec<&(String, String)> = agents.iter().collect();
    candidates.sort_by_key(|(name, _)| std::cmp::Reverse(name.len()));
    for (name, db_key) in candidates {
        if name.is_empty() {
            continue;
        }
        if let Some(rest) = event.strip_prefix(name.as_str()) {
            let is_outcome = ["passed", "failed", "error", "completed"]
                .iter()
                .any(|s| rest.strip_prefix(' ').is_some_and(|r| r.starts_with(s)));
            if is_outcome {
                return Some(db_key.clone());
            }
        }
    }
    None
}

/// Find the run of the given kind whose `started_at` is closest in time to
/// the activity entry's timestamp. The activity log entry and the agent run
/// row are written back-to-back, but their second-resolution timestamps can
/// differ, so an exact match is not guaranteed.
pub(in crate::app) fn nearest_agent_run<'a>(
    runs: &'a [crate::db::AgentRunEntry],
    db_key: &str,
    timestamp: &str,
) -> Option<&'a crate::db::AgentRunEntry> {
    const FMT: &str = "%Y-%m-%d %H:%M:%S";
    let target = chrono::NaiveDateTime::parse_from_str(timestamp, FMT).ok()?;
    runs.iter()
        .filter(|r| r.agent_kind == db_key)
        .filter_map(|r| {
            let t = chrono::NaiveDateTime::parse_from_str(&r.started_at, FMT).ok()?;
            Some(((t - target).num_seconds().abs(), r))
        })
        .min_by_key(|(delta, _)| *delta)
        .map(|(_, r)| r)
}

/// Format agent output, truncating if necessary.
/// Note: ANSI codes are already stripped by `build_completed_result` before persistence.
pub(in crate::app) fn format_agent_output(output: &str) -> String {
    if output.trim().is_empty() {
        "(no output)".to_string()
    } else if output.len() > 2000 {
        format!(
            "{}...\n(truncated, {} bytes total)",
            crate::app::truncate_str(output, 2000),
            output.len()
        )
    } else {
        output.to_string()
    }
}

/// Pick a deterministic badge color for a tag.
pub(in crate::app) fn tag_badge_color(tag: &str) -> egui::Color32 {
    let hash = tag.bytes().fold(5381u32, |acc, b| {
        acc.wrapping_mul(33).wrapping_add(b as u32)
    });
    let colors = [
        egui::Color32::from_rgb(38, 154, 108), // emerald
        egui::Color32::from_rgb(163, 68, 168), // vivid purple
        egui::Color32::from_rgb(206, 120, 36), // tangerine
        egui::Color32::from_rgb(44, 138, 186), // cerulean
        egui::Color32::from_rgb(210, 60, 78),  // coral
        egui::Color32::from_rgb(108, 72, 190), // violet
        egui::Color32::from_rgb(60, 120, 216), // royal blue
        egui::Color32::from_rgb(188, 82, 148), // magenta
    ];
    colors[(hash as usize) % colors.len()]
}

/// Pick a deterministic badge color based on the source label string.
pub(in crate::app) fn source_label_color(label: &str) -> egui::Color32 {
    let hash = label
        .bytes()
        .fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));
    let colors = [
        egui::Color32::from_rgb(60, 120, 216), // royal blue
        egui::Color32::from_rgb(163, 68, 168), // vivid purple
        egui::Color32::from_rgb(206, 120, 36), // tangerine
        egui::Color32::from_rgb(38, 154, 108), // emerald
        egui::Color32::from_rgb(210, 60, 78),  // coral
        egui::Color32::from_rgb(44, 138, 186), // cerulean
        egui::Color32::from_rgb(188, 82, 148), // magenta
        egui::Color32::from_rgb(108, 72, 190), // violet
    ];
    colors[(hash as usize) % colors.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::AgentRunEntry;

    fn agents() -> Vec<(String, String)> {
        vec![
            ("Cargo Check".to_string(), "custom_3".to_string()),
            ("Test Coverage".to_string(), "custom_7".to_string()),
            ("Test".to_string(), "test".to_string()),
            ("Lint".to_string(), "lint".to_string()),
        ]
    }

    #[test]
    fn detect_agent_event_matches_custom_names_and_db_keys() {
        let agents = agents();
        assert_eq!(
            detect_agent_event("Cargo Check passed (1.2s)", &agents),
            Some("custom_3".to_string())
        );
        assert_eq!(
            detect_agent_event("Test failed (300ms) \u{2014} error[E0308]", &agents),
            Some("test".to_string())
        );
        // Longest name wins: "Test Coverage" must not resolve to builtin "Test".
        assert_eq!(
            detect_agent_event("Test Coverage error (2.0s)", &agents),
            Some("custom_7".to_string())
        );
    }

    #[test]
    fn detect_agent_event_rejects_non_agent_events() {
        let agents = agents();
        assert_eq!(detect_agent_event("Moved to Review", &agents), None);
        assert_eq!(detect_agent_event("Tests are failing badly", &agents), None);
        // Outcome word must directly follow the agent name.
        assert_eq!(detect_agent_event("Lint run has passed", &agents), None);
    }

    fn run(kind: &str, started_at: &str, output: &str) -> AgentRunEntry {
        AgentRunEntry {
            agent_kind: kind.to_string(),
            status: "failed".to_string(),
            output: output.to_string(),
            duration_ms: 100,
            started_at: started_at.to_string(),
            command: String::new(),
            cue_id: Some(1),
        }
    }

    #[test]
    fn nearest_agent_run_prefers_closest_timestamp() {
        let runs = vec![
            run("test", "2026-07-16 10:00:05", "new"),
            run("lint", "2026-07-16 09:00:01", "other kind"),
            run("test", "2026-07-16 09:00:01", "old"),
        ];
        // Entry logged one second after the old run started.
        let found = nearest_agent_run(&runs, "test", "2026-07-16 09:00:02").unwrap();
        assert_eq!(found.output, "old");
    }

    #[test]
    fn nearest_agent_run_handles_unparsable_timestamps() {
        let runs = vec![run("test", "garbage", "a")];
        assert!(nearest_agent_run(&runs, "test", "2026-07-16 09:00:02").is_none());
        assert!(nearest_agent_run(&runs, "test", "garbage").is_none());
    }
}
