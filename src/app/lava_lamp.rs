//! Retro pixelated lava lamp overlay.
//!
//! A pixel-art lava lamp with animated wax blobs that rise and
//! fall inside the glass body. Shown as a floating overlay in the
//! bottom-right corner while a cue is running, then disappears.

use eframe::egui;

// -- Pixel grid dimensions --
const W: usize = 7;
const H: usize = 14;
const PX: f32 = 3.0;

/// Lamp frame outline (1 = frame pixel, 0 = empty or interior).
const FRAME: [[u8; W]; H] = [
    [0, 0, 1, 1, 1, 0, 0], //  0  cap top
    [0, 1, 1, 1, 1, 1, 0], //  1  cap
    [0, 1, 0, 0, 0, 1, 0], //  2  glass top
    [0, 1, 0, 0, 0, 1, 0], //  3  body narrow
    [1, 0, 0, 0, 0, 0, 1], //  4  body wide
    [1, 0, 0, 0, 0, 0, 1], //  5
    [1, 0, 0, 0, 0, 0, 1], //  6
    [1, 0, 0, 0, 0, 0, 1], //  7
    [1, 0, 0, 0, 0, 0, 1], //  8
    [0, 1, 0, 0, 0, 1, 0], //  9  narrowing
    [0, 1, 0, 0, 0, 1, 0], // 10  base
    [0, 1, 1, 1, 1, 1, 0], // 11  base plate
    [0, 0, 1, 1, 1, 0, 0], // 12  base bottom
    [0, 0, 0, 1, 0, 0, 0], // 13  foot
];

/// Interior mask (1 = inside glass where liquid and blobs are visible).
const INTERIOR: [[u8; W]; H] = [
    [0, 0, 0, 0, 0, 0, 0], //  0
    [0, 0, 0, 0, 0, 0, 0], //  1
    [0, 0, 1, 1, 1, 0, 0], //  2  narrow top
    [0, 0, 1, 1, 1, 0, 0], //  3
    [0, 1, 1, 1, 1, 1, 0], //  4  wide belly
    [0, 1, 1, 1, 1, 1, 0], //  5
    [0, 1, 1, 1, 1, 1, 0], //  6
    [0, 1, 1, 1, 1, 1, 0], //  7
    [0, 1, 1, 1, 1, 1, 0], //  8
    [0, 0, 1, 1, 1, 0, 0], //  9  narrow bottom
    [0, 0, 1, 1, 1, 0, 0], // 10
    [0, 0, 0, 0, 0, 0, 0], // 11
    [0, 0, 0, 0, 0, 0, 0], // 12
    [0, 0, 0, 0, 0, 0, 0], // 13
];

/// A wax blob definition with its own oscillation parameters.
struct Blob {
    period: f32,
    phase: f32,
    x_center: f32,
    radius: f32,
}

/// Four blobs with staggered periods and phases for organic motion.
const BLOBS: [Blob; 4] = [
    Blob {
        period: 22.0,
        phase: 0.0,
        x_center: 3.5,
        radius: 2.2,
    },
    Blob {
        period: 17.0,
        phase: 2.8,
        x_center: 2.8,
        radius: 1.6,
    },
    Blob {
        period: 13.0,
        phase: 5.5,
        x_center: 4.2,
        radius: 1.3,
    },
    Blob {
        period: 28.0,
        phase: 8.5,
        x_center: 3.0,
        radius: 1.0,
    },
];

/// Precomputed color palette for the lava lamp.
struct LampColors {
    frame_color: egui::Color32,
    cap_color: egui::Color32,
    liquid_bg: egui::Color32,
    blob_core: egui::Color32,
    blob_mid: egui::Color32,
    blob_dim: egui::Color32,
}

/// Compute the full color palette from the accent color and theme mode.
fn compute_colors(accent: egui::Color32, is_dark: bool) -> LampColors {
    let [ar, ag, ab, _] = accent.to_array();

    let frame_color = if is_dark {
        egui::Color32::from_rgb(130, 130, 145)
    } else {
        egui::Color32::from_rgb(90, 90, 105)
    };
    let cap_color = if is_dark {
        egui::Color32::from_rgb(150, 148, 155)
    } else {
        egui::Color32::from_rgb(110, 108, 115)
    };
    let liquid_bg = egui::Color32::from_rgb(ar / 8, ag / 8, ab / 8);
    let blob_core = egui::Color32::from_rgb(
        ar.saturating_add(60),
        ag.saturating_add(40),
        ab.saturating_add(30),
    );
    let blob_mid = accent;
    let blob_dim = egui::Color32::from_rgb(
        (ar as u16 * 2 / 3) as u8,
        (ag as u16 * 2 / 3) as u8,
        (ab as u16 * 2 / 3) as u8,
    );

    LampColors {
        frame_color,
        cap_color,
        liquid_bg,
        blob_core,
        blob_mid,
        blob_dim,
    }
}

/// Determine the color for a single pixel given its grid position.
///
/// Returns `Some(color)` when the pixel should be painted, or `None` for
/// empty background cells.
fn pixel_color(
    row: usize,
    col: usize,
    blob_positions: &[(f32, f32, f32)],
    colors: &LampColors,
) -> Option<egui::Color32> {
    if FRAME[row][col] == 1 {
        let color = if row <= 1 || row >= 11 {
            colors.cap_color
        } else {
            colors.frame_color
        };
        return Some(color);
    }

    if INTERIOR[row][col] != 1 {
        return None;
    }

    let cy = row as f32 + 0.5;
    let cx = col as f32 + 0.5;

    let mut intensity: f32 = 0.0;
    for &(bx, by, br) in blob_positions {
        let dx = cx - bx;
        let dy = cy - by;
        let dist = (dx * dx + dy * dy).sqrt();
        intensity = intensity.max((1.0 - dist / br).clamp(0.0, 1.0));
    }

    let color = if intensity > 0.6 {
        colors.blob_core
    } else if intensity > 0.3 {
        colors.blob_mid
    } else if intensity > 0.1 {
        colors.blob_dim
    } else {
        colors.liquid_bg
    };
    Some(color)
}

/// Total pixel dimensions of the lamp widget at the given scale.
pub fn size(scale: f32) -> (f32, f32) {
    let px = PX * scale;
    (W as f32 * px, H as f32 * px)
}

/// Paint the lava lamp at a specific position using the given painter.
///
/// Unlike `paint`, this does not allocate UI space — it just draws pixels
/// directly, making it suitable for overlaying on top of existing content.
pub fn paint_at(
    painter: &egui::Painter,
    ctx: &egui::Context,
    origin: egui::Pos2,
    accent: egui::Color32,
    is_dark: bool,
    scale: f32,
) {
    let px = PX * scale;
    let t = ctx.input(|i| i.time) as f32;

    let colors = compute_colors(accent, is_dark);

    let blob_positions: Vec<(f32, f32, f32)> = BLOBS
        .iter()
        .map(|b| {
            let y_norm = ((t / b.period * std::f32::consts::TAU + b.phase).sin() + 1.0) / 2.0;
            let y = 2.5 + y_norm * 7.5;
            let x = b.x_center + 0.4 * (t * 0.2 + b.phase * 2.0).sin();
            (x, y, b.radius)
        })
        .collect();

    for row in 0..H {
        for col in 0..W {
            if let Some(color) = pixel_color(row, col, &blob_positions, &colors) {
                let px_rect = egui::Rect::from_min_size(
                    origin + egui::vec2(col as f32 * px, row as f32 * px),
                    egui::vec2(px, px),
                );
                painter.rect_filled(px_rect, 0.0, color);
            }
        }
    }

    ctx.request_repaint_after(std::time::Duration::from_millis(500));
}
