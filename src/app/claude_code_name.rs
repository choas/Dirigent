//! Animated pixel-art Claude character overlay.
//!
//! A pixelated creature with antennae and eyes, all motion and color
//! snapped to the pixel grid like the lava lamp overlay.

use eframe::egui;

const PX: f32 = 3.0;

const W: usize = 9;
const H: usize = 11;
const ARM_LEN: usize = 2;

// 1 = body, 2 = eye
const CHARACTER: [[u8; W]; H] = [
    [0, 0, 0, 1, 0, 1, 0, 0, 0], // antennae
    [0, 0, 0, 1, 0, 1, 0, 0, 0],
    [0, 1, 1, 1, 1, 1, 1, 1, 0], // head
    [0, 1, 1, 1, 1, 1, 1, 1, 0],
    [0, 1, 2, 1, 1, 1, 2, 1, 0], // eyes
    [0, 1, 1, 1, 1, 1, 1, 1, 0],
    [1, 1, 1, 1, 1, 1, 1, 1, 1], // body (widest)
    [1, 1, 1, 1, 1, 1, 1, 1, 1],
    [1, 1, 1, 1, 1, 1, 1, 1, 1],
    [0, 1, 1, 1, 1, 1, 1, 1, 0], // lower
    [0, 1, 1, 0, 0, 0, 1, 1, 0], // feet
];

/// Discrete brightness levels for the pixelated breath effect.
const BREATH_LEVELS: [f32; 3] = [0.7, 0.85, 1.0];

struct BodyColors {
    body: egui::Color32,
    eye: egui::Color32,
    eye_closed: egui::Color32,
    text: egui::Color32,
}

fn compute_colors(accent: egui::Color32, is_dark: bool, breath_step: usize) -> BodyColors {
    let [ar, ag, ab, _] = accent.to_array();
    let b = BREATH_LEVELS[breath_step % BREATH_LEVELS.len()];
    let body = egui::Color32::from_rgb(
        (ar as f32 * b).min(255.0) as u8,
        (ag as f32 * b).min(255.0) as u8,
        (ab as f32 * b).min(255.0) as u8,
    );
    let eye = if is_dark {
        egui::Color32::from_rgb(30, 25, 25)
    } else {
        egui::Color32::from_rgb(40, 35, 35)
    };
    let eye_closed = body;
    let text = egui::Color32::from_rgb(
        (ar as f32 * 0.85).min(255.0) as u8,
        (ag as f32 * 0.85).min(255.0) as u8,
        (ab as f32 * 0.85).min(255.0) as u8,
    );
    BodyColors {
        body,
        eye,
        eye_closed,
        text,
    }
}

/// Quantize time to discrete steps, producing a stepped tick counter.
fn tick(t: f32, interval: f32) -> i32 {
    (t / interval).floor() as i32
}

pub fn size(scale: f32, display_name: &str) -> (f32, f32) {
    let px = PX * scale;
    let char_w = (W + 2 * ARM_LEN) as f32 * px;
    let char_h = H as f32 * px;
    let text_h = if display_name.is_empty() {
        0.0
    } else {
        10.0 * scale + 4.0 * scale
    };
    let w = char_w.max(display_name.chars().count() as f32 * 6.0 * scale);
    (w, char_h + text_h)
}

pub fn paint_at(
    painter: &egui::Painter,
    ctx: &egui::Context,
    origin: egui::Pos2,
    accent: egui::Color32,
    is_dark: bool,
    scale: f32,
    display_name: &str,
) {
    let px = PX * scale;
    let t = ctx.input(|i| i.time) as f32;

    // Breath cycles through discrete brightness levels (~1.5s per step)
    let breath_step = (tick(t, 1.5).rem_euclid(BREATH_LEVELS.len() as i32)) as usize;
    let colors = compute_colors(accent, is_dark, breath_step);

    // Blink: eyes close for one full tick every ~8 ticks (4s)
    let blink_tick = tick(t, 0.5).rem_euclid(8);
    let eyes_closed = blink_tick == 7;

    let mut char_origin = origin;
    let full_char_w = (W + 2 * ARM_LEN) as f32 * px;

    if !display_name.is_empty() {
        let text_size = 10.0 * scale;
        let text_w = display_name.chars().count() as f32 * 6.0 * scale;
        let total_w = full_char_w.max(text_w);
        let text_x = origin.x + (total_w - text_w) / 2.0;
        painter.text(
            egui::pos2(text_x, origin.y),
            egui::Align2::LEFT_TOP,
            display_name,
            egui::FontId::monospace(text_size),
            colors.text,
        );
        char_origin.y += text_size + 4.0 * scale;
        let body_w = W as f32 * px;
        char_origin.x = origin.x + (total_w - body_w) / 2.0;
    } else {
        char_origin.x = origin.x + ARM_LEN as f32 * px;
    }

    // Bob snapped to whole-pixel offsets: cycles 0, -1, 0, +1 pixels
    let bob_pattern: [i32; 4] = [0, -1, 0, 1];
    let bob_idx = (tick(t, 0.6).rem_euclid(4)) as usize;
    let bob = bob_pattern[bob_idx] as f32 * px;
    char_origin.y += bob;

    for row in 0..H {
        for col in 0..W {
            let cell = CHARACTER[row][col];
            if cell == 0 {
                continue;
            }
            let color = if cell == 2 {
                if eyes_closed {
                    colors.eye_closed
                } else {
                    colors.eye
                }
            } else {
                colors.body
            };
            let px_rect = egui::Rect::from_min_size(
                char_origin + egui::vec2(col as f32 * px, row as f32 * px),
                egui::vec2(px, px),
            );
            painter.rect_filled(px_rect, 0.0, color);
        }
    }

    // Arms wave in discrete pixel steps: offset cycles -1, 0, +1, 0
    let arm_pattern: [i32; 4] = [-1, 0, 1, 0];
    let left_idx = (tick(t, 0.5).rem_euclid(4)) as usize;
    let right_idx = (tick(t, 0.5).rem_euclid(4) + 2).rem_euclid(4) as usize;
    let shoulder_row = 7;

    for seg in 0..ARM_LEN {
        let left_off = arm_pattern[(left_idx + seg) % 4] as f32 * px;
        painter.rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(
                    char_origin.x - (seg as f32 + 1.0) * px,
                    char_origin.y + shoulder_row as f32 * px + left_off,
                ),
                egui::vec2(px, px),
            ),
            0.0,
            colors.body,
        );

        let right_off = arm_pattern[(right_idx + seg) % 4] as f32 * px;
        painter.rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(
                    char_origin.x + W as f32 * px + seg as f32 * px,
                    char_origin.y + shoulder_row as f32 * px + right_off,
                ),
                egui::vec2(px, px),
            ),
            0.0,
            colors.body,
        );
    }

    ctx.request_repaint_after(std::time::Duration::from_millis(500));
}
