//! Thunderdome (Hardcore / Gabba) pixel overlay.
//!
//! A grinning gabber skull headbangs to a relentless ~190 BPM kick drum while
//! lightning bolts crack on either side on every beat — an homage to the
//! Thunderdome hardcore/gabber aesthetic. Used as one of the "running"
//! animations.

use eframe::egui;

const PX: f32 = 2.0;

// Total grid dimensions (skull + room for the headbang bounce + side bolts).
const W: usize = 26;
const H: usize = 22;

// Skull sprite footprint within the grid.
const SKULL_W: usize = 16;
const SKULL_H: usize = 16;
const SKULL_COL: usize = 5; // left column of the skull within the grid
const SKULL_ROW: usize = 1; // resting top row of the skull within the grid

/// Skull sprite. 0 = empty, 1 = bone, 2 = eye socket, 3 = nose.
const SKULL: [[u8; SKULL_W]; SKULL_H] = [
    [0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0], //  0 cranium top
    [0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0], //  1
    [0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0], //  2
    [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1], //  3
    [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1], //  4
    [1, 1, 2, 2, 2, 1, 1, 1, 1, 1, 1, 2, 2, 2, 1, 1], //  5 eye sockets
    [1, 1, 2, 2, 2, 1, 1, 1, 1, 1, 1, 2, 2, 2, 1, 1], //  6
    [1, 1, 2, 2, 2, 1, 1, 1, 1, 1, 1, 2, 2, 2, 1, 1], //  7
    [1, 1, 1, 1, 1, 1, 1, 3, 3, 1, 1, 1, 1, 1, 1, 1], //  8 nose
    [0, 1, 1, 1, 1, 1, 1, 3, 3, 1, 1, 1, 1, 1, 1, 0], //  9
    [0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0], // 10 cheeks
    [0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0], // 11 jaw narrows
    [0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0], // 12 teeth top
    [0, 0, 0, 1, 0, 1, 0, 1, 1, 0, 1, 0, 1, 0, 0, 0], // 13 teeth gaps
    [0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0], // 14
    [0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0], // 15 chin
];

const BOLT_W: usize = 4;
const BOLT_H: usize = 11;

/// Lightning bolt drawn to the left of the skull (mirrored for the right).
const BOLT: [[u8; BOLT_W]; BOLT_H] = [
    [0, 0, 1, 1],
    [0, 0, 1, 0],
    [0, 1, 1, 0],
    [0, 1, 0, 0],
    [1, 1, 1, 0],
    [0, 1, 1, 0],
    [0, 0, 1, 0],
    [0, 1, 1, 0],
    [0, 1, 0, 0],
    [1, 1, 0, 0],
    [1, 0, 0, 0],
];

const BOLT_ROW: usize = 3; // top row of the side bolts within the grid

/// Gabber tempo: ~190 BPM. One kick every BEAT seconds.
const BEAT: f32 = 60.0 / 190.0;

struct DomeColors {
    bone: egui::Color32,
    highlight: egui::Color32,
    shadow: egui::Color32,
    socket: egui::Color32,
    eye_glow: egui::Color32,
    bolt: egui::Color32,
}

fn compute_colors(accent: egui::Color32, is_dark: bool) -> DomeColors {
    let [ar, ag, ab, _] = accent.to_array();
    let bone = accent;
    let highlight = egui::Color32::from_rgb(
        ar.saturating_add(35),
        ag.saturating_add(35),
        ab.saturating_add(35),
    );
    let shadow = egui::Color32::from_rgb(
        ar.saturating_sub(45),
        ag.saturating_sub(45),
        ab.saturating_sub(45),
    );
    let socket = if is_dark {
        egui::Color32::from_rgb(18, 16, 16)
    } else {
        egui::Color32::from_rgb(40, 30, 30)
    };
    // The kick-drum eye flash: a hot hardcore red.
    let eye_glow = egui::Color32::from_rgb(255, 60, 40);
    // Electric "thunder" yellow; muted slightly on light themes so it reads.
    let bolt = if is_dark {
        egui::Color32::from_rgb(255, 232, 64)
    } else {
        egui::Color32::from_rgb(225, 170, 20)
    };
    DomeColors {
        bone,
        highlight,
        shadow,
        socket,
        eye_glow,
        bolt,
    }
}

/// Total pixel dimensions of the widget at the given scale.
pub fn size(scale: f32) -> (f32, f32) {
    let px = PX * scale;
    (W as f32 * px, H as f32 * px)
}

/// Paint the headbanging Thunderdome skull at `origin`.
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

    // Beat timing: phase 0.0 == the kick hit, ramping to 1.0 at the next beat.
    let beat_index = (t / BEAT).floor() as i64;
    let beat_phase = (t / BEAT).fract() as f32;

    // Punchy downward headbang: the skull slams down on the kick and eases back.
    let bounce_rows = 4.0 * (-(beat_phase * 6.5)).exp();
    let skull_origin = origin + egui::vec2(0.0, bounce_rows * px);

    // A short flash window right after each kick.
    let flashing = beat_phase < 0.18;

    // Draw the skull with edge-detection shading (highlight on top edges,
    // shadow on bottom edges) for a chunky pixel-art look.
    for row in 0..SKULL_H {
        for col in 0..SKULL_W {
            let cell = SKULL[row][col];
            if cell == 0 {
                continue;
            }
            let above = if row > 0 { SKULL[row - 1][col] } else { 0 };
            let below = if row + 1 < SKULL_H {
                SKULL[row + 1][col]
            } else {
                0
            };
            let color = match cell {
                // Eye sockets glow red on the kick, otherwise sit dark.
                2 => {
                    if flashing {
                        colors.eye_glow
                    } else {
                        colors.socket
                    }
                }
                3 => colors.socket,
                _ if above == 0 => colors.highlight,
                _ if below == 0 => colors.shadow,
                _ => colors.bone,
            };
            let gx = (SKULL_COL + col) as f32;
            let gy = (SKULL_ROW + row) as f32;
            let rect = egui::Rect::from_min_size(
                skull_origin + egui::vec2(gx * px, gy * px),
                egui::vec2(px, px),
            );
            painter.rect_filled(rect, 0.0, color);
        }
    }

    // Lightning cracks on the beat. Alternate which side leads for variety,
    // but flash both for a proper thunderclap.
    if flashing {
        let lead_left = beat_index % 2 == 0;
        for row in 0..BOLT_H {
            for col in 0..BOLT_W {
                if BOLT[row][col] == 0 {
                    continue;
                }
                let gy = (BOLT_ROW + row) as f32;
                // Left bolt.
                let lx = col as f32;
                let l_color = if lead_left {
                    colors.bolt
                } else {
                    colors.shadow
                };
                painter.rect_filled(
                    egui::Rect::from_min_size(
                        origin + egui::vec2(lx * px, gy * px),
                        egui::vec2(px, px),
                    ),
                    0.0,
                    l_color,
                );
                // Right bolt (mirrored).
                let rx = (W - 1 - col) as f32;
                let r_color = if lead_left {
                    colors.shadow
                } else {
                    colors.bolt
                };
                painter.rect_filled(
                    egui::Rect::from_min_size(
                        origin + egui::vec2(rx * px, gy * px),
                        egui::vec2(px, px),
                    ),
                    0.0,
                    r_color,
                );
            }
        }
    }

    ctx.request_repaint_after(std::time::Duration::from_millis(30));
}
