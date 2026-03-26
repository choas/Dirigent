use std::time::Instant;

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
    scheduled_when: Option<Instant>,
) -> String {
    if is_queued {
        return "\u{23F3} Queued".to_string();
    }
    if let Some(when) = scheduled_when {
        let remaining = when.saturating_duration_since(Instant::now());
        let secs = remaining.as_secs();
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

/// Detect if an activity event is an agent event and return its kind label.
pub(in crate::app) fn detect_agent_kind(event: &str) -> Option<String> {
    let is_agent_event =
        event.contains("passed") || event.contains("failed") || event.contains("error");
    if !is_agent_event {
        return None;
    }
    ["Format", "Lint", "Build", "Test"]
        .iter()
        .find(|k| event.starts_with(*k))
        .map(|k| k.to_string())
}

/// Format agent output, truncating if necessary.
pub(in crate::app) fn format_agent_output(output: &str) -> String {
    if output.len() > 2000 {
        format!(
            "{}...\n(truncated, {} bytes total)",
            crate::app::truncate_str(output, 2000),
            output.len()
        )
    } else if output.trim().is_empty() {
        "(no output)".to_string()
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
