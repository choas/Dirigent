//! Animated pixel-art Claude sparkle overlay.
//!
//! A pulsing 4-pointed star with orbiting sparkle particles.
//! Shown as a floating overlay in the bottom-right corner of the
//! cue pool, as an alternative to the lava lamp.

use eframe::egui;

const PX: f32 = 3.0;

const W: usize = 11;
const H: usize = 11;

/// 4-pointed star shape; values 1–3 are brightness levels.
const STAR: [[u8; W]; H] = [
    [0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 1, 3, 1, 0, 0, 0, 0],
    [0, 0, 0, 0, 1, 3, 1, 0, 0, 0, 0],
    [0, 0, 1, 1, 2, 3, 2, 1, 1, 0, 0],
    [1, 2, 3, 3, 3, 3, 3, 3, 3, 2, 1],
    [0, 0, 1, 1, 2, 3, 2, 1, 1, 0, 0],
    [0, 0, 0, 0, 1, 3, 1, 0, 0, 0, 0],
    [0, 0, 0, 0, 1, 3, 1, 0, 0, 0, 0],
    [0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0],
];

struct Sparkle {
    radius: f32,
    period: f32,
    phase: f32,
    size: f32,
    blink_speed: f32,
}

const SPARKLES: [Sparkle; 5] = [
    Sparkle {
        radius: 7.0,
        period: 3.2,
        phase: 0.0,
        size: 1.0,
        blink_speed: 2.0,
    },
    Sparkle {
        radius: 6.0,
        period: 4.8,
        phase: 2.1,
        size: 0.7,
        blink_speed: 3.0,
    },
    Sparkle {
        radius: 7.5,
        period: 5.5,
        phase: 4.2,
        size: 0.5,
        blink_speed: 1.5,
    },
    Sparkle {
        radius: 5.5,
        period: 2.8,
        phase: 1.0,
        size: 0.8,
        blink_speed: 2.5,
    },
    Sparkle {
        radius: 6.5,
        period: 6.0,
        phase: 3.5,
        size: 0.6,
        blink_speed: 1.8,
    },
];

struct StarColors {
    levels: [egui::Color32; 4],
    sparkle_base: [f32; 3],
}

fn compute_colors(accent: egui::Color32, is_dark: bool, breath: f32) -> StarColors {
    let [ar, ag, ab, _] = accent.to_array();
    let dim = if is_dark { 0.25_f32 } else { 0.35 };

    let levels = [
        egui::Color32::TRANSPARENT,
        egui::Color32::from_rgb(
            (ar as f32 * dim * breath).min(255.0) as u8,
            (ag as f32 * dim * breath).min(255.0) as u8,
            (ab as f32 * dim * breath).min(255.0) as u8,
        ),
        egui::Color32::from_rgb(
            (ar as f32 * 0.7 * breath).min(255.0) as u8,
            (ag as f32 * 0.7 * breath).min(255.0) as u8,
            (ab as f32 * 0.7 * breath).min(255.0) as u8,
        ),
        egui::Color32::from_rgb(
            (ar as f32 * breath + 40.0 * breath).min(255.0) as u8,
            (ag as f32 * breath + 20.0 * breath).min(255.0) as u8,
            (ab as f32 * breath + 15.0 * breath).min(255.0) as u8,
        ),
    ];

    StarColors {
        levels,
        sparkle_base: [ar as f32, ag as f32, ab as f32],
    }
}

pub fn size(scale: f32, _display_name: &str) -> (f32, f32) {
    let px = PX * scale;
    let side = (W as f32 + 14.0) * px;
    (side, side)
}

pub fn paint_at(
    painter: &egui::Painter,
    ctx: &egui::Context,
    origin: egui::Pos2,
    accent: egui::Color32,
    is_dark: bool,
    scale: f32,
    _display_name: &str,
) {
    let px = PX * scale;
    let t = ctx.input(|i| i.time) as f32;

    let breath = 0.75 + 0.25 * (t * 1.5).sin();
    let colors = compute_colors(accent, is_dark, breath);

    let margin = 7.0 * px;

    for row in 0..H {
        for col in 0..W {
            let level = STAR[row][col] as usize;
            if level > 0 {
                let px_rect = egui::Rect::from_min_size(
                    origin + egui::vec2(margin + col as f32 * px, margin + row as f32 * px),
                    egui::vec2(px, px),
                );
                painter.rect_filled(px_rect, 0.0, colors.levels[level]);
            }
        }
    }

    let center = origin + egui::vec2(margin + W as f32 * px / 2.0, margin + H as f32 * px / 2.0);

    let [br, bg, bb] = colors.sparkle_base;
    for sparkle in &SPARKLES {
        let angle = t / sparkle.period * std::f32::consts::TAU + sparkle.phase;
        let r = sparkle.radius * px;
        let pos = egui::pos2(center.x + angle.cos() * r, center.y + angle.sin() * r);

        let fade = ((t * sparkle.blink_speed + sparkle.phase).sin() * 0.5 + 0.5) * breath;
        let sparkle_color = egui::Color32::from_rgb(
            ((br + 60.0) * fade).min(255.0) as u8,
            ((bg + 40.0) * fade).min(255.0) as u8,
            ((bb + 30.0) * fade).min(255.0) as u8,
        );

        let s = px * sparkle.size;
        let sparkle_rect = egui::Rect::from_center_size(pos, egui::vec2(s, s));
        painter.rect_filled(sparkle_rect, 0.0, sparkle_color);
    }

    ctx.request_repaint_after(std::time::Duration::from_millis(50));
}
