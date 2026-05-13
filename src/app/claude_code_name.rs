//! Animated pixel-art Claude character overlay.
//!
//! A cute pixel creature with antennae and eyes, gently breathing
//! and blinking, with the display name rendered above.

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

struct BodyColors {
    body: egui::Color32,
    eye: egui::Color32,
    eye_closed: egui::Color32,
    text: egui::Color32,
}

fn compute_colors(accent: egui::Color32, is_dark: bool, breath: f32) -> BodyColors {
    let [ar, ag, ab, _] = accent.to_array();
    let body = egui::Color32::from_rgb(
        (ar as f32 * breath).min(255.0) as u8,
        (ag as f32 * breath).min(255.0) as u8,
        (ab as f32 * breath).min(255.0) as u8,
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

pub fn size(scale: f32, display_name: &str) -> (f32, f32) {
    let px = PX * scale;
    let char_w = (W + 2 * ARM_LEN) as f32 * px;
    let char_h = H as f32 * px;
    let text_h = if display_name.is_empty() {
        0.0
    } else {
        10.0 * scale + 4.0 * scale
    };
    let w = char_w.max(display_name.len() as f32 * 6.0 * scale);
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

    let breath = 0.8 + 0.2 * (t * 1.2).sin();
    let colors = compute_colors(accent, is_dark, breath);

    // Blink: eyes close briefly every ~4 seconds
    let blink_cycle = t % 4.0;
    let eyes_closed = blink_cycle > 3.8 && blink_cycle < 3.95;

    let mut char_origin = origin;
    let full_char_w = (W + 2 * ARM_LEN) as f32 * px;

    if !display_name.is_empty() {
        let text_size = 10.0 * scale;
        let text_w = display_name.len() as f32 * 6.0 * scale;
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

    // Gentle bob
    let bob = (t * 0.8).sin() * px * 0.5;
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

    // Animated arms waving in opposite phases
    let arm_speed = 2.5;
    let left_wave = (t * arm_speed).sin();
    let right_wave = (t * arm_speed + std::f32::consts::PI).sin();
    let shoulder_row = 7.0;

    for seg in 0..ARM_LEN {
        let factor = (seg + 1) as f32 / ARM_LEN as f32;

        let left_y_off = left_wave * px * 1.5 * factor;
        painter.rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(
                    char_origin.x - (seg as f32 + 1.0) * px,
                    char_origin.y + shoulder_row * px + left_y_off,
                ),
                egui::vec2(px, px),
            ),
            0.0,
            colors.body,
        );

        let right_y_off = right_wave * px * 1.5 * factor;
        painter.rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(
                    char_origin.x + W as f32 * px + seg as f32 * px,
                    char_origin.y + shoulder_row * px + right_y_off,
                ),
                egui::vec2(px, px),
            ),
            0.0,
            colors.body,
        );
    }

    ctx.request_repaint_after(std::time::Duration::from_millis(50));
}
