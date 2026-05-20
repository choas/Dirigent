//! Desert-running pixel dino overlay (homage to the Chrome offline T-Rex).
//!
//! A tiny pixel dinosaur trots across a sand line, jumping to clear stones
//! that scroll past underneath it. Used as one of the "running" animations.

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
    stone: egui::Color32,
}

fn compute_colors(accent: egui::Color32, is_dark: bool) -> DinoColors {
    let [ar, ag, ab, _] = accent.to_array();
    // Classic Chrome T-Rex tone — neutral gray so the dino doesn't end up
    // tinted like whatever the theme's accent happens to be.
    let body = if is_dark {
        egui::Color32::from_rgb(205, 200, 190)
    } else {
        egui::Color32::from_rgb(83, 83, 83)
    };
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
    let stone = if is_dark {
        egui::Color32::from_rgb(160, 140, 110)
    } else {
        egui::Color32::from_rgb(105, 85, 65)
    };
    DinoColors {
        body,
        eye,
        sand,
        pebble,
        stone,
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

// Stones spawn at the right edge and scroll left across the ground at this
// speed. Spacing is staggered with per-spawn jitter so the rhythm doesn't
// feel metronomic.
const STONE_SPEED: f32 = 18.0;
const STONE_SPACING: f32 = 1.7;
const STONE_SPAWN_COL: f32 = W as f32;
// Column at which a stone is "under" the dino's feet — used both to time the
// jump apex and as the horizontal centre of the collision window.
const DINO_FRONT_COL: f32 = 5.5;

const JUMP_DURATION: f32 = 0.55;
const JUMP_HEIGHT: f32 = 5.0;

#[derive(Clone, Copy)]
struct Stone {
    /// Current column (fractional) of the stone's left edge.
    col_f: f32,
    /// Height in grid cells (1 or 2).
    height: u8,
    /// Wall-clock time at which this stone reaches `DINO_FRONT_COL`.
    collide_t: f32,
}

/// Stones currently on-screen, newest first.
fn active_stones(t: f32) -> Vec<Stone> {
    let mut stones = Vec::new();
    // The +1 catches the next stone that hasn't quite spawned yet so we can
    // still pre-trigger the jump for it.
    let current_bucket = (t / STONE_SPACING).floor() as i32 + 1;
    for back in 0..4 {
        let bucket = current_bucket - back;
        if bucket < 0 {
            continue;
        }
        let h = hash_u32(bucket as u32);
        // Up to ±0.3s of jitter on the spawn time within the bucket.
        let jitter = ((h & 0xFF) as f32 / 255.0 - 0.5) * 0.6;
        let spawn_t = bucket as f32 * STONE_SPACING + jitter;
        let age = t - spawn_t;
        let col_f = STONE_SPAWN_COL - age * STONE_SPEED;
        if col_f < -3.0 || col_f > STONE_SPAWN_COL + 0.5 {
            continue;
        }
        let height = if (h >> 8) & 0b11 == 0 { 2 } else { 1 };
        let collide_t = spawn_t + (STONE_SPAWN_COL - DINO_FRONT_COL) / STONE_SPEED;
        stones.push(Stone {
            col_f,
            height,
            collide_t,
        });
    }
    stones
}

/// Vertical jump offset in pixel-grid units (0 = on ground, positive = airborne).
///
/// Jumps are anchored to upcoming stones — the apex lines up with the moment a
/// stone passes under the dino's feet.
fn jump_offset(t: f32, stones: &[Stone]) -> f32 {
    for stone in stones {
        let jump_start = stone.collide_t - JUMP_DURATION * 0.5;
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

    let stones = active_stones(t);
    let jump_px = jump_offset(t, &stones);
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

    // Stones sitting on top of the sand — these are the obstacles the dino
    // jumps over. Drawn at sub-cell precision so they scroll smoothly.
    for stone in &stones {
        let stone_w_px = 2.0 * px;
        let stone_h_px = stone.height as f32 * px;
        let top_row = GROUND_ROW - stone.height as usize;
        let x = origin.x + stone.col_f * px;
        let y = origin.y + top_row as f32 * px;
        let rect = egui::Rect::from_min_size(
            egui::pos2(x, y),
            egui::vec2(stone_w_px, stone_h_px),
        );
        painter.rect_filled(rect, 0.0, colors.stone);
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
