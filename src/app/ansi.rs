use eframe::egui;

/// Overrides for the ANSI red/green colors that Claude Code's TUI uses for
/// diff lines. When present, foreground / background red and green
/// (standard `31/32/41/42` and bright `91/92/101/102`) are remapped to the
/// user's selected diff color scheme so the inline PTY diff matches the
/// rest of the app.
#[derive(Clone, Default)]
pub struct DiffAnsiOverrides {
    pub addition_fg: Option<egui::Color32>,
    pub deletion_fg: Option<egui::Color32>,
    pub addition_bg: Option<egui::Color32>,
    pub deletion_bg: Option<egui::Color32>,
}

/// Parse a string with ANSI SGR escape sequences into an egui `LayoutJob`.
///
/// Recognises the SGR subset that Claude Code's TUI emits via
/// [`claude_pty::Event::TuiScreen::lines_ansi`]: reset (`0`), italics (`3`),
/// underline (`4`), the 8 standard / 8 bright foreground (`30-37`, `90-97`)
/// and background (`40-47`, `100-107`) colors, the 256-color palette
/// (`38;5;N`, `48;5;N`), and true-color (`38;2;R;G;B`, `48;2;R;G;B`).
/// Bold and inverse are accepted but not visually applied (egui's
/// `TextFormat` has no bold flag).
///
/// Non-SGR CSI sequences (e.g. cursor movement) and OSC sequences are
/// stripped so the rendered text doesn't contain escape garbage.
///
/// `overrides` remaps ANSI red/green to the user's diff color scheme so
/// Claude's inline PTY diff output matches the Settings page.
pub fn ansi_to_layout_job(
    text: &str,
    font_id: egui::FontId,
    default_color: egui::Color32,
    overrides: &DiffAnsiOverrides,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let base = egui::TextFormat {
        font_id: font_id.clone(),
        color: default_color,
        ..Default::default()
    };
    let mut current = base.clone();
    let mut buf = String::new();
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if c != '\x1b' {
            buf.push(c);
            continue;
        }
        let Some(&next) = chars.peek() else {
            break;
        };
        if next == '[' {
            chars.next();
            let mut params = String::new();
            let mut final_byte = '\0';
            for c2 in chars.by_ref() {
                if !c2.is_ascii_digit() && c2 != ';' && c2 != '?' && c2 != ':' {
                    final_byte = c2;
                    break;
                }
                params.push(c2);
            }
            if final_byte == 'm' {
                if !buf.is_empty() {
                    job.append(&buf, 0.0, current.clone());
                    buf.clear();
                }
                apply_sgr(&params, &mut current, &base, overrides);
            }
        } else if next == ']' {
            chars.next();
            while let Some(c2) = chars.next() {
                if c2 == '\x07' {
                    break;
                }
                if c2 == '\x1b' && chars.peek() == Some(&'\\') {
                    chars.next();
                    break;
                }
            }
        } else {
            chars.next();
        }
    }
    if !buf.is_empty() {
        job.append(&buf, 0.0, current);
    }
    job
}

fn apply_sgr(
    params: &str,
    current: &mut egui::TextFormat,
    base: &egui::TextFormat,
    overrides: &DiffAnsiOverrides,
) {
    let nums: Vec<u32> = if params.is_empty() {
        vec![0]
    } else {
        params.split(';').map(|s| s.parse().unwrap_or(0)).collect()
    };
    let mut i = 0;
    while i < nums.len() {
        match nums[i] {
            0 => *current = base.clone(),
            1 => { /* bold — egui TextFormat has no bold flag */ }
            3 => current.italics = true,
            4 => current.underline = egui::Stroke::new(1.0, current.color),
            7 => std::mem::swap(&mut current.color, &mut current.background),
            22 => { /* reset bold — no-op */ }
            23 => current.italics = false,
            24 => current.underline = egui::Stroke::NONE,
            27 => std::mem::swap(&mut current.color, &mut current.background),
            31 => current.color = overrides.deletion_fg.unwrap_or_else(|| ansi_standard(1)),
            32 => current.color = overrides.addition_fg.unwrap_or_else(|| ansi_standard(2)),
            n @ 30..=37 => current.color = ansi_standard(n - 30),
            38 => {
                if let Some(color) = parse_ext_color(&nums, &mut i) {
                    current.color = color;
                }
            }
            39 => current.color = base.color,
            41 => current.background = overrides.deletion_bg.unwrap_or_else(|| ansi_standard(1)),
            42 => current.background = overrides.addition_bg.unwrap_or_else(|| ansi_standard(2)),
            n @ 40..=47 => current.background = ansi_standard(n - 40),
            48 => {
                if let Some(color) = parse_ext_color(&nums, &mut i) {
                    current.background = color;
                }
            }
            49 => current.background = base.background,
            91 => current.color = overrides.deletion_fg.unwrap_or_else(|| ansi_bright(1)),
            92 => current.color = overrides.addition_fg.unwrap_or_else(|| ansi_bright(2)),
            n @ 90..=97 => current.color = ansi_bright(n - 90),
            101 => current.background = overrides.deletion_bg.unwrap_or_else(|| ansi_bright(1)),
            102 => current.background = overrides.addition_bg.unwrap_or_else(|| ansi_bright(2)),
            n @ 100..=107 => current.background = ansi_bright(n - 100),
            _ => {}
        }
        i += 1;
    }
}

/// Parse the trailing operand of an `38`/`48` SGR (`;5;N` or `;2;R;G;B`).
/// Advances `i` past the consumed sub-parameters; the caller still increments
/// past the leading `38`/`48` itself.
fn parse_ext_color(nums: &[u32], i: &mut usize) -> Option<egui::Color32> {
    let mode = *nums.get(*i + 1)?;
    if mode == 5 {
        let idx = *nums.get(*i + 2)? as u8;
        *i += 2;
        Some(ansi_256(idx))
    } else if mode == 2 {
        let r = *nums.get(*i + 2)? as u8;
        let g = *nums.get(*i + 3)? as u8;
        let b = *nums.get(*i + 4)? as u8;
        *i += 4;
        Some(egui::Color32::from_rgb(r, g, b))
    } else {
        None
    }
}

fn ansi_standard(n: u32) -> egui::Color32 {
    // VS Code-ish dark palette — readable on both light and dark backgrounds.
    match n {
        0 => egui::Color32::from_rgb(0, 0, 0),
        1 => egui::Color32::from_rgb(205, 49, 49),
        2 => egui::Color32::from_rgb(13, 188, 121),
        3 => egui::Color32::from_rgb(229, 229, 16),
        4 => egui::Color32::from_rgb(36, 114, 200),
        5 => egui::Color32::from_rgb(188, 63, 188),
        6 => egui::Color32::from_rgb(17, 168, 205),
        7 => egui::Color32::from_rgb(229, 229, 229),
        _ => egui::Color32::GRAY,
    }
}

fn ansi_bright(n: u32) -> egui::Color32 {
    match n {
        0 => egui::Color32::from_rgb(102, 102, 102),
        1 => egui::Color32::from_rgb(241, 76, 76),
        2 => egui::Color32::from_rgb(35, 209, 139),
        3 => egui::Color32::from_rgb(245, 245, 67),
        4 => egui::Color32::from_rgb(59, 142, 234),
        5 => egui::Color32::from_rgb(214, 112, 214),
        6 => egui::Color32::from_rgb(41, 184, 219),
        7 => egui::Color32::from_rgb(229, 229, 229),
        _ => egui::Color32::GRAY,
    }
}

fn ansi_256(idx: u8) -> egui::Color32 {
    match idx {
        0..=7 => ansi_standard(idx as u32),
        8..=15 => ansi_bright((idx - 8) as u32),
        16..=231 => {
            let v = idx - 16;
            let r = v / 36;
            let g = (v % 36) / 6;
            let b = v % 6;
            let scale = |c: u8| if c == 0 { 0 } else { 55 + c * 40 };
            egui::Color32::from_rgb(scale(r), scale(g), scale(b))
        }
        232..=255 => {
            let v = 8 + (idx - 232) * 10;
            egui::Color32::from_rgb(v, v, v)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job_text(job: &egui::text::LayoutJob) -> String {
        job.text.clone()
    }

    fn no_overrides() -> DiffAnsiOverrides {
        DiffAnsiOverrides::default()
    }

    #[test]
    fn plain_text_passes_through() {
        let job = ansi_to_layout_job(
            "hello world",
            egui::FontId::monospace(12.0),
            egui::Color32::WHITE,
            &no_overrides(),
        );
        assert_eq!(job_text(&job), "hello world");
        assert_eq!(job.sections.len(), 1);
    }

    #[test]
    fn standard_fg_color_creates_section() {
        let job = ansi_to_layout_job(
            "\x1b[31mred\x1b[0m plain",
            egui::FontId::monospace(12.0),
            egui::Color32::WHITE,
            &no_overrides(),
        );
        assert_eq!(job_text(&job), "red plain");
        assert!(job.sections.len() >= 2);
        assert_eq!(job.sections[0].format.color, ansi_standard(1));
        assert_eq!(job.sections[1].format.color, egui::Color32::WHITE);
    }

    #[test]
    fn truecolor_fg_parses_rgb() {
        let job = ansi_to_layout_job(
            "\x1b[38;2;10;20;30mhi\x1b[0m",
            egui::FontId::monospace(12.0),
            egui::Color32::WHITE,
            &no_overrides(),
        );
        assert_eq!(job_text(&job), "hi");
        assert_eq!(
            job.sections[0].format.color,
            egui::Color32::from_rgb(10, 20, 30)
        );
    }

    #[test]
    fn extended_256_color_parses() {
        let job = ansi_to_layout_job(
            "\x1b[38;5;208morange\x1b[0m",
            egui::FontId::monospace(12.0),
            egui::Color32::WHITE,
            &no_overrides(),
        );
        assert_eq!(job_text(&job), "orange");
        assert_eq!(job.sections[0].format.color, ansi_256(208));
    }

    #[test]
    fn unknown_csi_is_stripped() {
        let job = ansi_to_layout_job(
            "before\x1b[2Kafter",
            egui::FontId::monospace(12.0),
            egui::Color32::WHITE,
            &no_overrides(),
        );
        assert_eq!(job_text(&job), "beforeafter");
    }

    #[test]
    fn osc_sequence_is_stripped() {
        let job = ansi_to_layout_job(
            "x\x1b]0;title\x07y",
            egui::FontId::monospace(12.0),
            egui::Color32::WHITE,
            &no_overrides(),
        );
        assert_eq!(job_text(&job), "xy");
    }

    #[test]
    fn reset_returns_to_default_color() {
        let job = ansi_to_layout_job(
            "\x1b[31mred\x1b[0mplain",
            egui::FontId::monospace(12.0),
            egui::Color32::WHITE,
            &no_overrides(),
        );
        let last = job.sections.last().unwrap();
        assert_eq!(last.format.color, egui::Color32::WHITE);
    }

    #[test]
    fn diff_overrides_remap_red_green() {
        let overrides = DiffAnsiOverrides {
            addition_fg: Some(egui::Color32::from_rgb(0, 0, 200)),
            deletion_fg: Some(egui::Color32::from_rgb(200, 200, 0)),
            ..Default::default()
        };
        let job = ansi_to_layout_job(
            "\x1b[31mdel\x1b[0m\x1b[32madd\x1b[0m",
            egui::FontId::monospace(12.0),
            egui::Color32::WHITE,
            &overrides,
        );
        assert_eq!(
            job.sections[0].format.color,
            egui::Color32::from_rgb(200, 200, 0)
        );
        assert_eq!(
            job.sections[1].format.color,
            egui::Color32::from_rgb(0, 0, 200)
        );
    }
}
