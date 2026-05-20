//! Desert-running pixel dino overlay (homage to the Chrome offline T-Rex).
//!
//! A tiny pixel dinosaur trots across a sand line, randomly jumping over
//! invisible obstacles. Used as one of the "running" animations.

use eframe::egui;

const PX: f32 = 3.0;

// Total grid dimensions (dino + ground + pebbles).
const W: usize = 20;
const H: usize = 14;

// Dino sprite footprint within the grid.
const DINO_W: usize = 14;
const DINO_BASE_ROWS: usize = 9;

/// Dino body without the legs. 1 = body, 2 = eye.
const DINO_BASE: [[u8; DINO_W]; DINO_BASE_ROWS] = [
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1], //  0  head top
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1], //  1
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 2, 1, 1], //  2  eye
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1], //  3
    [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1], //  4  neck
    [1, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0], //  5  tail tip + back
    [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0], //  6  body
    [0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0], //  7  belly
    [0, 0, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0], //  8  lower belly
];

/// Running leg frame A (left foot forward).
const LEGS_A: [[u8; DINO_W]; 2] = [
    [0, 0, 1, 1, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0],
    [0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
];

/// Running leg frame B (right foot forward).
const LEGS_B: [[u8; DINO_W]; 2] = [
    [0, 0, 0, 1, 1, 0, 1, 1, 0, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0],
];

/// Legs tucked up while airborne.
const LEGS_JUMP: [[u8; DINO_W]; 2] = [
    [0, 0, 1, 1, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
];

const GROUND_ROW: usize = 11;

struct DinoColors {
    body: egui::Color32,
    eye: egui::Color32,
    sand: egui::Color32,
    pebble: egui::Color32,
}

fn compute_colors(accent: egui::Color32, is_dark: bool) -> DinoColors {
    let [ar, ag, ab, _] = accent.to_array();
    let body = accent;
    let eye = if is_dark {
        egui::Color32::from_rgb(20, 18, 18)
    } else {
        egui::Color32::from_rgb(245, 240, 235)
    };
    // Warm sand tone, biased away from the accent so the ground reads as desert.
    let sand = if is_dark {
        egui::Color32::from_rgb(
            ar.saturating_add(20).max(150),
            ag.saturating_add(10).max(125),
            ab.saturating_sub(30).min(95),
        )
    } else {
        egui::Color32::from_rgb(180, 150, 100)
    };
    let pebble = if is_dark {
        egui::Color32::from_rgb(120, 100, 70)
    } else {
        egui::Color32::from_rgb(135, 110, 80)
    };
    DinoColors {
        body,
        eye,
        sand,
        pebble,
    }
}

/// Deterministic 32-bit hash of an integer "time bucket" seed.
fn hash_u32(seed: u32) -> u32 {
    let mut x = seed.wrapping_mul(2654435761).wrapping_add(0x9E3779B9);
    x ^= x >> 13;
    x = x.wrapping_mul(0x85EBCA6B);
    x ^= x >> 16;
    x
}

/// Vertical jump offset in pixel-grid units (0 = on ground, positive = airborne).
///
/// Jumps fire pseudo-randomly so the dino never lands in a perfect rhythm.
fn jump_offset(t: f32) -> f32 {
    const JUMP_DURATION: f32 = 0.55;
    const JUMP_HEIGHT: f32 = 5.0;

    let now_sec = t.floor() as i32;
    // Look at the current and previous second to catch an in-flight jump.
    for back in 0..2 {
        let s = now_sec - back;
        if s < 0 {
            continue;
        }
        let h = hash_u32(s as u32);
        // ~33% chance to start a jump within this second.
        if h % 3 != 0 {
            continue;
        }
        let frac = ((h >> 8) & 0xFFFF) as f32 / 65536.0;
        let jump_start = s as f32 + frac * 0.6;
        let elapsed = t - jump_start;
        if elapsed >= 0.0 && elapsed < JUMP_DURATION {
            let p = elapsed / JUMP_DURATION;
            return JUMP_HEIGHT * 4.0 * p * (1.0 - p);
        }
    }
    0.0
}

/// Total pixel dimensions of the widget at the given scale.
pub fn size(scale: f32) -> (f32, f32) {
    let px = PX * scale;
    (W as f32 * px, H as f32 * px)
}

/// Paint the desert-running dino at `origin`.
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

    let jump_px = jump_offset(t);
    let airborne = jump_px > 0.01;

    // Two-frame run cycle at ~8 Hz.
    let run_phase = (t * 8.0).sin() > 0.0;
    let legs: &[[u8; DINO_W]; 2] = if airborne {
        &LEGS_JUMP
    } else if run_phase {
        &LEGS_A
    } else {
        &LEGS_B
    };

    // Dino sits with its feet on GROUND_ROW. Subtract the jump offset to lift it.
    let dino_origin = origin + egui::vec2(0.0, -jump_px * px);

    // Body
    for row in 0..DINO_BASE_ROWS {
        for col in 0..DINO_W {
            let cell = DINO_BASE[row][col];
            if cell == 0 {
                continue;
            }
            let color = if cell == 2 { colors.eye } else { colors.body };
            let rect = egui::Rect::from_min_size(
                dino_origin + egui::vec2(col as f32 * px, row as f32 * px),
                egui::vec2(px, px),
            );
            painter.rect_filled(rect, 0.0, color);
        }
    }

    // Legs (two rows under the body)
    for (i, leg_row) in legs.iter().enumerate() {
        let row_idx = DINO_BASE_ROWS + i;
        for col in 0..DINO_W {
            if leg_row[col] == 0 {
                continue;
            }
            let rect = egui::Rect::from_min_size(
                dino_origin + egui::vec2(col as f32 * px, row_idx as f32 * px),
                egui::vec2(px, px),
            );
            painter.rect_filled(rect, 0.0, colors.body);
        }
    }

    // Ground line (always drawn at fixed origin — the dino is what moves)
    for col in 0..W {
        let rect = egui::Rect::from_min_size(
            origin + egui::vec2(col as f32 * px, GROUND_ROW as f32 * px),
            egui::vec2(px, px),
        );
        painter.rect_filled(rect, 0.0, colors.sand);
    }

    // Scrolling pebbles below the ground line.
    let scroll = (t * 18.0) as i32;
    for row_off in 0..2 {
        let row_idx = GROUND_ROW + 1 + row_off;
        if row_idx >= H {
            break;
        }
        for col in 0..W {
            // Pseudo-random pebble pattern that scrolls left over time.
            let key = ((col as i32 + scroll) as u32).wrapping_mul(73)
                ^ (row_off as u32).wrapping_mul(919);
            let h = hash_u32(key);
            // Sparser on the lower row.
            let threshold = if row_off == 0 { 0x16 } else { 0x09 };
            if h & 0xFF < threshold {
                let rect = egui::Rect::from_min_size(
                    origin + egui::vec2(col as f32 * px, row_idx as f32 * px),
                    egui::vec2(px, px),
                );
                painter.rect_filled(rect, 0.0, colors.pebble);
            }
        }
    }

    ctx.request_repaint_after(std::time::Duration::from_millis(50));
}
