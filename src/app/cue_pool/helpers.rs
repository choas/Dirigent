use std::collections::{BTreeSet, HashMap};

use eframe::egui;

use crate::db::{Cue, CueStatus};
use crate::settings;

/// Build the heading text showing cue counts.
pub(in crate::app) fn build_heading_text(cues: &[Cue]) -> String {
    let inbox = cues.iter().filter(|c| c.status == CueStatus::Inbox).count();
    let review = cues
        .iter()
        .filter(|c| c.status == CueStatus::Review)
        .count();
    let counts: Vec<String> = [
        if inbox > 0 {
            Some(format!("{} inbox", inbox))
        } else {
            None
        },
        if review > 0 {
            Some(format!("{} review", review))
        } else {
            None
        },
    ]
    .into_iter()
    .flatten()
    .collect();
    if counts.is_empty() {
        "Cues".to_string()
    } else {
        format!("Cues ({})", counts.join(", "))
    }
}

pub(super) fn render_cue_pool_buttons(
    ui: &mut egui::Ui,
    playbook: &[settings::Play],
) -> (Option<String>, bool, bool) {
    let mut selected_play_prompt = None;
    let mut custom_cue_requested = false;
    let mut import_requested = false;

    let plus_btn = ui.button("+").on_hover_text("Playbook");
    if ui
        .button("\u{2193}")
        .on_hover_text("Import from document")
        .clicked()
    {
        import_requested = true;
    }
    egui::Popup::menu(&plus_btn)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
        .show(|ui| {
            ui.set_min_width(200.0);
            ui.label(egui::RichText::new("Playbook").strong());
            ui.separator();
            for play in playbook {
                if ui.selectable_label(false, &play.name).clicked() {
                    selected_play_prompt = Some(play.prompt.clone());
                }
            }
            if !playbook.is_empty() {
                ui.separator();
            }
            if ui.selectable_label(false, "+ Custom cue...").clicked() {
                custom_cue_requested = true;
            }
        });

    (selected_play_prompt, custom_cue_requested, import_requested)
}

/// Collect unique source labels from cues and settings.
pub(in crate::app) fn collect_unique_labels(
    cues: &[Cue],
    sources: &[crate::settings::SourceConfig],
) -> Vec<String> {
    let mut labels = BTreeSet::new();
    for c in cues {
        if let Some(ref label) = c.source_label {
            labels.insert(label.clone());
        }
    }
    for s in sources {
        if s.enabled {
            labels.insert(s.label.clone());
        }
    }
    labels.into_iter().collect()
}

/// Group cues by status in a single pass (replaces 6× filter_cues_by_status_and_source).
/// Cues within each group are in reverse order (newest first), matching the old behaviour.
pub(super) fn group_cues_by_status<'a>(
    cues: &'a [Cue],
    source_filter: &Option<String>,
) -> HashMap<CueStatus, Vec<&'a Cue>> {
    let mut map: HashMap<CueStatus, Vec<&Cue>> = HashMap::new();
    for cue in cues.iter().rev() {
        if let Some(ref filter) = source_filter {
            if cue.source_label.as_deref() != Some(filter.as_str()) {
                continue;
            }
        }
        map.entry(cue.status).or_default().push(cue);
    }
    map
}

/// Build section header text for a cue status column.
pub(super) fn build_section_header(
    status: CueStatus,
    count: usize,
    archived_total: usize,
) -> String {
    if status == CueStatus::Archived && archived_total > count {
        format!("{} ({}/{})", status.label(), count, archived_total)
    } else {
        format!("{} ({})", status.label(), count)
    }
}

/// Format the import message.
pub(super) fn format_import_message(
    new_count: usize,
    updated_count: usize,
    filename: &str,
) -> String {
    match (new_count, updated_count) {
        (0, 0) => format!("No changes from \"{}\"", filename),
        (n, 0) => format!("Imported {} new cue(s) from \"{}\"", n, filename),
        (0, u) => {
            format!("Updated {} cue(s) from \"{}\"", u, filename)
        }
        (n, u) => format!(
            "Imported {} new, updated {} cue(s) from \"{}\"",
            n, u, filename
        ),
    }
}

/// Build the commit subject line for a "Commit All" action.
pub(super) fn build_commit_all_subject(review_cues: &[&Cue]) -> String {
    if review_cues.len() == 1 {
        build_single_cue_subject(review_cues[0])
    } else {
        build_multi_cue_subject(review_cues)
    }
}

fn build_single_cue_subject(cue: &Cue) -> String {
    let first_line = cue
        .text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or_else(|| cue.text.lines().next().unwrap_or(&cue.text));
    let trimmed = first_line.trim();
    let trimmed = if trimmed.is_empty() {
        let fallback = cue.text.trim();
        if fallback.is_empty() {
            ""
        } else {
            fallback
        }
    } else {
        trimmed
    };
    if trimmed.is_empty() {
        return "chore: dirigent commit".to_string();
    }
    let commit_type = crate::git::detect_commit_type(trimmed);
    let prefix = format!("{}: ", commit_type);
    let allowed = 72 - prefix.len();
    if trimmed.len() > allowed {
        format!(
            "{}{}...",
            prefix,
            crate::app::truncate_str(trimmed, allowed - 3)
        )
    } else {
        format!("{}{}", prefix, trimmed)
    }
}

fn build_multi_cue_subject(review_cues: &[&Cue]) -> String {
    let short_names: Vec<&str> = review_cues
        .iter()
        .filter_map(|c| {
            let first = c
                .text
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or_else(|| c.text.lines().next().unwrap_or(&c.text));
            let trimmed = first.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        })
        .collect();
    if short_names.is_empty() {
        return format!("chore: {} cues", review_cues.len());
    }
    // Detect type from the combined cue texts.
    let combined_text = short_names.join(", ");
    let commit_type = crate::git::detect_commit_type(&combined_text);
    let prefix = format!("{}: ", commit_type);
    let allowed = 72 - prefix.len();
    if combined_text.len() <= allowed {
        return format!("{}{}", prefix, combined_text);
    }
    let truncated = crate::app::truncate_str(&combined_text, allowed - 3);
    if truncated.is_empty() {
        format!("{}{} cues", prefix, review_cues.len())
    } else {
        format!("{}{}...", prefix, truncated)
    }
}

/// Parse a schedule duration string like "5m", "2h", "30s" into a `Duration`.
pub(super) fn parse_schedule_duration(input: &str) -> Option<std::time::Duration> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    let (num_str, suffix) = if let Some(s) = input.strip_suffix('m') {
        (s, 'm')
    } else if let Some(s) = input.strip_suffix('h') {
        (s, 'h')
    } else if let Some(s) = input.strip_suffix('s') {
        (s, 's')
    } else {
        // Default to minutes if no suffix
        (input, 'm')
    };
    let num: u64 = num_str.trim().parse().ok()?;
    if num == 0 {
        return None;
    }
    match suffix {
        's' => Some(std::time::Duration::from_secs(num)),
        'm' => Some(std::time::Duration::from_secs(num * 60)),
        'h' => Some(std::time::Duration::from_secs(num * 3600)),
        _ => None,
    }
}
