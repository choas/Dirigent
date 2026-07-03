use std::path::PathBuf;

use eframe::egui;

/// Expand a leading `~` to the user's home directory.
pub fn expand_tilde(raw: &str) -> PathBuf {
    if raw == "~" || raw.starts_with("~/") || raw.starts_with("~\\") {
        match dirs::home_dir() {
            Some(home) => {
                if raw == "~" {
                    home
                } else {
                    home.join(&raw[2..])
                }
            }
            None => PathBuf::from(raw),
        }
    } else {
        PathBuf::from(raw)
    }
}

/// Strip ANSI escape sequences (colors, bold, etc.) from a string.
pub fn strip_ansi(s: &str) -> String {
    let stripped = strip_ansi_escapes::strip(s);
    String::from_utf8_lossy(&stripped).into_owned()
}

/// Format a duration in milliseconds to a human-readable string.
pub fn format_duration_ms(ms: u64) -> String {
    if ms < 1000 {
        format!("{}ms", ms)
    } else {
        format!("{:.1}s", ms as f64 / 1000.0)
    }
}

pub(crate) fn pixel_grid_size(width: usize, height: usize, px: f32, scale: f32) -> (f32, f32) {
    let scaled = px * scale;
    (width as f32 * scaled, height as f32 * scaled)
}

pub(crate) struct GridRect {
    pub(crate) col: f32,
    pub(crate) row: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
}

impl GridRect {
    pub(crate) fn cell(col: usize, row: usize) -> Self {
        Self {
            col: col as f32,
            row: row as f32,
            width: 1.0,
            height: 1.0,
        }
    }
}

pub(crate) fn paint_grid_cell(
    painter: &egui::Painter,
    origin: egui::Pos2,
    col: usize,
    row: usize,
    px: f32,
    color: egui::Color32,
) {
    paint_grid_rect(painter, origin, GridRect::cell(col, row), px, color);
}

pub(crate) fn paint_grid_rect(
    painter: &egui::Painter,
    origin: egui::Pos2,
    grid_rect: GridRect,
    px: f32,
    color: egui::Color32,
) {
    let rect = egui::Rect::from_min_size(
        origin + egui::vec2(grid_rect.col * px, grid_rect.row * px),
        egui::vec2(grid_rect.width * px, grid_rect.height * px),
    );
    painter.rect_filled(rect, 0.0, color);
}

pub(crate) fn paint_grid_row(
    painter: &egui::Painter,
    origin: egui::Pos2,
    row: usize,
    columns: std::ops::Range<usize>,
    px: f32,
    color: egui::Color32,
) {
    for col in columns {
        paint_grid_cell(painter, origin, col, row, px, color);
    }
}

pub(crate) fn vertical_neighbors<const WIDTH: usize, const HEIGHT: usize>(
    grid: &[[u8; WIDTH]; HEIGHT],
    row: usize,
    col: usize,
) -> (u8, u8) {
    let above = row.checked_sub(1).map_or(0, |r| grid[r][col]);
    let below = grid.get(row + 1).map_or(0, |r| r[col]);
    (above, below)
}

pub(crate) fn paint_u8_grid<const WIDTH: usize, const HEIGHT: usize>(
    painter: &egui::Painter,
    origin: egui::Pos2,
    grid: &[[u8; WIDTH]; HEIGHT],
    px: f32,
    color_for: impl FnMut(usize, usize, u8) -> egui::Color32,
) {
    paint_u8_grid_at(painter, origin, grid, px, 0.0, 0.0, color_for);
}

pub(crate) fn paint_u8_grid_at<const WIDTH: usize, const HEIGHT: usize>(
    painter: &egui::Painter,
    origin: egui::Pos2,
    grid: &[[u8; WIDTH]; HEIGHT],
    px: f32,
    col_offset: f32,
    row_offset: f32,
    mut color_for: impl FnMut(usize, usize, u8) -> egui::Color32,
) {
    for (row, cells) in grid.iter().enumerate() {
        for (col, cell) in cells.iter().copied().enumerate() {
            if cell == 0 {
                continue;
            }
            paint_grid_rect(
                painter,
                origin,
                GridRect {
                    col: col_offset + col as f32,
                    row: row_offset + row as f32,
                    width: 1.0,
                    height: 1.0,
                },
                px,
                color_for(row, col, cell),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- strip_ansi tests --

    #[test]
    fn strip_ansi_plain_text() {
        assert_eq!(strip_ansi("hello"), "hello");
    }

    #[test]
    fn strip_ansi_colored_text() {
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m"), "red");
    }

    #[test]
    fn strip_ansi_bold_and_color() {
        assert_eq!(strip_ansi("\x1b[1;33mwarning\x1b[0m"), "warning");
    }

    #[test]
    fn strip_ansi_empty_string() {
        assert_eq!(strip_ansi(""), "");
    }

    #[test]
    fn strip_ansi_no_escape_sequences() {
        assert_eq!(strip_ansi("just plain text"), "just plain text");
    }

    #[test]
    fn strip_ansi_non_ascii_preserved() {
        assert_eq!(strip_ansi("\x1b[32mcafé 日本語\x1b[0m"), "café 日本語");
    }

    // -- format_duration_ms tests --

    #[test]
    fn format_duration_ms_zero() {
        assert_eq!(format_duration_ms(0), "0ms");
    }

    #[test]
    fn format_duration_ms_under_one_second() {
        assert_eq!(format_duration_ms(500), "500ms");
        assert_eq!(format_duration_ms(999), "999ms");
    }

    #[test]
    fn format_duration_ms_exactly_one_second() {
        assert_eq!(format_duration_ms(1000), "1.0s");
    }

    #[test]
    fn format_duration_ms_seconds() {
        assert_eq!(format_duration_ms(1500), "1.5s");
        assert_eq!(format_duration_ms(60000), "60.0s");
    }

    #[test]
    fn format_duration_ms_boundary() {
        assert_eq!(format_duration_ms(999), "999ms");
        assert_eq!(format_duration_ms(1000), "1.0s");
    }

    // -- expand_tilde tests --

    #[test]
    fn expand_tilde_no_tilde() {
        assert_eq!(expand_tilde("/usr/local"), PathBuf::from("/usr/local"));
    }

    #[test]
    fn expand_tilde_with_subpath() {
        let result = expand_tilde("~/Documents");
        assert!(result.ends_with("Documents"));
        assert_ne!(result, PathBuf::from("~/Documents"));
    }

    #[test]
    fn expand_tilde_bare_tilde() {
        let result = expand_tilde("~");
        assert_ne!(result, PathBuf::from("~"));
    }

    #[test]
    fn expand_tilde_windows_separator() {
        let result = expand_tilde("~\\Documents");
        assert!(result.ends_with("Documents"));
        assert_ne!(result, PathBuf::from("~\\Documents"));
    }

    #[test]
    fn expand_tilde_no_expansion_for_middle_tilde() {
        assert_eq!(expand_tilde("/home/~user"), PathBuf::from("/home/~user"));
    }
}
