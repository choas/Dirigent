//! Desert-running pixel dino overlay (homage to the Chrome offline T-Rex).
//!
//! A tiny pixel dinosaur trots across a sand line, jumping to clear stones
//! that scroll past underneath it. Used as one of the "running" animations.

use eframe::egui;

const PX: f32 = 2.0;

// Total grid dimensions (dino + ground + pebbles).
const W: usize = 30;
const H: usize = 22;

// Dino sprite footprint within the grid.
const DINO_W: usize = 22;
const DINO_BASE_ROWS: usize = 14;

/// Dino body without the legs. 1 = body, 2 = eye.
const DINO_BASE: [[u8; DINO_W]; DINO_BASE_ROWS] = [
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 0, 0, 0, 0, 0, 0,
    ], //  0  head bump
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0,
    ], //  1  head top
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0,
    ], //  2  head
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 1, 1, 1, 1, 1, 0, 0, 0, 0,
    ], //  3  eye
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0,
    ], //  4  head
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0,
    ], //  5  jaw
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 1, 1, 0, 0, 0,
    ], //  6  mouth + lower jaw
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0,
    ], //  7  neck
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0,
    ], //  8  neck wider
    [
        1, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0,
    ], //  9  tail + body
    [
        0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 1, 1, 0, 0, 0,
    ], // 10  body + arm
    [
        0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 1, 1, 0, 0, 0,
    ], // 11  body + arm
    [
        0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0,
    ], // 12  belly
    [
        0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ], // 13  hip
];

/// Running leg frame A (back foot forward).
const LEGS_A: [[u8; DINO_W]; 2] = [
    [
        0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ],
    [
        0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ],
];

/// Running leg frame B (front foot forward).
const LEGS_B: [[u8; DINO_W]; 2] = [
    [
        0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ],
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ],
];

/// Legs tucked up while airborne.
const LEGS_JUMP: [[u8; DINO_W]; 2] = [
    [
        0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ],
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ],
];

const GROUND_ROW: usize = 17;

struct DinoColors {
    body: egui::Color32,
    highlight: egui::Color32,
    shadow: egui::Color32,
    eye: egui::Color32,
    sand: egui::Color32,
    pebble: egui::Color32,
    stone: egui::Color32,
}

fn compute_colors(accent: egui::Color32, is_dark: bool) -> DinoColors {
    let [ar, ag, ab, _] = accent.to_array();
    let body = accent;
    let highlight = egui::Color32::from_rgb(
        ar.saturating_add(35),
        ag.saturating_add(35),
        ab.saturating_add(35),
    );
    let shadow = egui::Color32::from_rgb(
        ar.saturating_sub(40),
        ag.saturating_sub(40),
        ab.saturating_sub(40),
    );
    let eye = if is_dark {
        egui::Color32::from_rgb(20, 18, 18)
    } else {
        egui::Color32::from_rgb(245, 240, 235)
    };
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
        highlight,
        shadow,
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

const STONE_SPEED: f32 = 27.0;
const STONE_SPACING: f32 = 1.7;
const STONE_SPAWN_COL: f32 = W as f32;
const DINO_FRONT_COL: f32 = 8.5;

const JUMP_DURATION: f32 = 0.55;
const JUMP_HEIGHT: f32 = 7.5;

#[derive(Clone, Copy)]
struct Stone {
    /// Current column (fractional) of the stone's left edge.
    col_f: f32,
    /// Height in grid cells (2 or 3).
    height: u8,
    /// Wall-clock time at which this stone reaches `DINO_FRONT_COL`.
    collide_t: f32,
}

/// Stones currently on-screen, newest first.
fn active_stones(t: f32) -> Vec<Stone> {
    let mut stones = Vec::new();
    let current_bucket = (t / STONE_SPACING).floor() as i32 + 1;
    for back in 0..4 {
        let bucket = current_bucket - back;
        if bucket < 0 {
            continue;
        }
        let h = hash_u32(bucket as u32);
        let jitter = ((h & 0xFF) as f32 / 255.0 - 0.5) * 0.6;
        let spawn_t = bucket as f32 * STONE_SPACING + jitter;
        let age = t - spawn_t;
        let col_f = STONE_SPAWN_COL - age * STONE_SPEED;
        if !(-4.0..=STONE_SPAWN_COL + 0.5).contains(&col_f) {
            continue;
        }
        let height = if (h >> 8) & 0b11 == 0 { 3 } else { 2 };
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
fn jump_offset(t: f32, stones: &[Stone]) -> f32 {
    for stone in stones {
        let jump_start = stone.collide_t - JUMP_DURATION * 0.5;
        let elapsed = t - jump_start;
        if (0.0..JUMP_DURATION).contains(&elapsed) {
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

    let run_phase = (t * 8.0).sin() > 0.0;
    let legs: &[[u8; DINO_W]; 2] = if airborne {
        &LEGS_JUMP
    } else if run_phase {
        &LEGS_A
    } else {
        &LEGS_B
    };

    let dino_origin = origin + egui::vec2(0.0, -jump_px * px);

    // Build combined body+legs grid for edge-detection shading.
    const TOTAL_ROWS: usize = DINO_BASE_ROWS + 2;
    let mut grid = [[0u8; DINO_W]; TOTAL_ROWS];
    let mut r = 0;
    while r < DINO_BASE_ROWS {
        grid[r] = DINO_BASE[r];
        r += 1;
    }
    grid[DINO_BASE_ROWS] = legs[0];
    grid[DINO_BASE_ROWS + 1] = legs[1];

    #[allow(clippy::needless_range_loop)]
    for row in 0..TOTAL_ROWS {
        for col in 0..DINO_W {
            let cell = grid[row][col];
            if cell == 0 {
                continue;
            }
            let above = if row > 0 { grid[row - 1][col] } else { 0 };
            let below = if row + 1 < TOTAL_ROWS {
                grid[row + 1][col]
            } else {
                0
            };
            let color = if cell == 2 {
                colors.eye
            } else if above == 0 {
                colors.highlight
            } else if below == 0 {
                colors.shadow
            } else {
                colors.body
            };
            let rect = egui::Rect::from_min_size(
                dino_origin + egui::vec2(col as f32 * px, row as f32 * px),
                egui::vec2(px, px),
            );
            painter.rect_filled(rect, 0.0, color);
        }
    }

    // Ground line
    for col in 0..W {
        let rect = egui::Rect::from_min_size(
            origin + egui::vec2(col as f32 * px, GROUND_ROW as f32 * px),
            egui::vec2(px, px),
        );
        painter.rect_filled(rect, 0.0, colors.sand);
    }

    // Stones (obstacles)
    for stone in &stones {
        let stone_w_px = 3.0 * px;
        let stone_h_px = stone.height as f32 * px;
        let top_row = GROUND_ROW - stone.height as usize;
        let x = origin.x + stone.col_f * px;
        let y = origin.y + top_row as f32 * px;
        let rect = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(stone_w_px, stone_h_px));
        painter.rect_filled(rect, 0.0, colors.stone);
    }

    // Scrolling pebbles below the ground line.
    let scroll = (t * 27.0) as i32;
    for row_off in 0..3 {
        let row_idx = GROUND_ROW + 1 + row_off;
        if row_idx >= H {
            break;
        }
        for col in 0..W {
            let key = ((col as i32 + scroll) as u32).wrapping_mul(73)
                ^ (row_off as u32).wrapping_mul(919);
            let h = hash_u32(key);
            let threshold = match row_off {
                0 => 0x16,
                1 => 0x09,
                _ => 0x05,
            };
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
