//! Thunderdome (Hardcore / Gabba) pixel overlay.
//!
//! A grinning gabber skull headbangs to a relentless ~190 BPM kick drum while
//! lightning bolts crack on either side on every beat — an homage to the
//! Thunderdome hardcore/gabber aesthetic. Used as one of the "running"
//! animations.

use eframe::egui;

use super::util;

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
const SKULL: [&str; SKULL_H] = [
    "0000111111110000", //  0 cranium top
    "0011111111111100", //  1
    "0111111111111110", //  2
    "1111111111111111", //  3
    "1111111111111111", //  4
    "1122211111122211", //  5 eye sockets
    "1122211111122211", //  6
    "1122211111122211", //  7
    "1111111331111111", //  8 nose
    "0111111331111110", //  9
    "0111111111111110", // 10 cheeks
    "0011111111111100", // 11 jaw narrows
    "0001111111111000", // 12 teeth top
    "0001010110101000", // 13 teeth gaps
    "0001111111111000", // 14
    "0000111111110000", // 15 chin
];

const BOLT_W: usize = 4;
const BOLT_H: usize = 11;

/// Lightning bolt drawn to the left of the skull (mirrored for the right).
const BOLT: [&str; BOLT_H] = [
    "0011", //  0
    "0010", //  1
    "0110", //  2
    "0100", //  3
    "1110", //  4
    "0110", //  5
    "0010", //  6
    "0110", //  7
    "0100", //  8
    "1100", //  9
    "1000", // 10
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
    util::pixel_grid_size(W, H, PX, scale)
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
    let beat_phase = (t / BEAT).fract();

    // Punchy downward headbang: the skull slams down on the kick and eases back.
    let bounce_rows = 4.0 * (-(beat_phase * 6.5)).exp();
    let skull_origin = origin + egui::vec2(0.0, bounce_rows * px);

    // A short flash window right after each kick.
    let flashing = beat_phase < 0.18;

    draw_skull(painter, skull_origin, px, flashing, &colors);

    // Lightning cracks on the beat.
    if flashing {
        let lead_left = beat_index % 2 == 0;
        draw_bolts(painter, origin, px, lead_left, &colors);
    }

    ctx.request_repaint_after(std::time::Duration::from_millis(30));
}

/// Draw the skull with edge-detection shading (highlight on top edges,
/// shadow on bottom edges) for a chunky pixel-art look.
fn draw_skull(
    painter: &egui::Painter,
    skull_origin: egui::Pos2,
    px: f32,
    flashing: bool,
    colors: &DomeColors,
) {
    let origin = skull_origin + egui::vec2(SKULL_COL as f32 * px, SKULL_ROW as f32 * px);
    util::paint_pixel_grid(painter, origin, SKULL_W, SKULL_H, px, |row, col| {
        let cell = util::mask_value(&SKULL, row, col);
        if cell == 0 {
            return None;
        }
        let (above, below) = util::mask_vertical_neighbors(&SKULL, row, col);
        Some(skull_cell_color(cell, above, below, flashing, colors))
    });
}

/// Shading color for one skull cell given its vertical neighbours.
fn skull_cell_color(
    cell: u8,
    above: u8,
    below: u8,
    flashing: bool,
    colors: &DomeColors,
) -> egui::Color32 {
    match cell {
        // Eye sockets glow red on the kick, otherwise sit dark.
        2 if flashing => colors.eye_glow,
        2 | 3 => colors.socket,
        _ if above == 0 => colors.highlight,
        _ if below == 0 => colors.shadow,
        _ => colors.bone,
    }
}

/// Draw both side bolts. Alternate which side leads for variety,
/// but flash both for a proper thunderclap.
fn draw_bolts(
    painter: &egui::Painter,
    origin: egui::Pos2,
    px: f32,
    lead_left: bool,
    colors: &DomeColors,
) {
    let (l_color, r_color) = if lead_left {
        (colors.bolt, colors.shadow)
    } else {
        (colors.shadow, colors.bolt)
    };
    let bolt_origin = origin + egui::vec2(0.0, BOLT_ROW as f32 * px);
    util::paint_pixel_grid(painter, bolt_origin, BOLT_W, BOLT_H, px, |row, col| {
        (util::mask_value(&BOLT, row, col) != 0).then_some(l_color)
    });

    let mirrored_origin = origin + egui::vec2((W - BOLT_W) as f32 * px, BOLT_ROW as f32 * px);
    util::paint_pixel_grid(painter, mirrored_origin, BOLT_W, BOLT_H, px, |row, col| {
        (util::mask_value(&BOLT, row, BOLT_W - 1 - col) != 0).then_some(r_color)
    });
}
