//! Pixel-art Jujutsu martial artist overlay.
//!
//! A tiny pixel figure cycles through jujutsu stances while a
//! sweeping circle (enso) rotates behind it. Shown as a floating
//! overlay in the bottom-right corner while a cue is running.

use eframe::egui;

const PX: f32 = 2.0;

const W: usize = 18;
const H: usize = 20;

// -- Stance A: ready stance (feet apart, arms guard) --
// 1 = body, 2 = belt
const STANCE_A: [[u8; W]; H] = [
    [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0], //  0  head top
    [0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0], //  1  head
    [0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0], //  2  head
    [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0], //  3  neck
    [0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0], //  4  shoulders
    [0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0], //  5  upper torso
    [0, 0, 0, 0, 1, 1, 0, 1, 1, 1, 1, 0, 1, 1, 0, 0, 0, 0], //  6  arms out
    [0, 0, 0, 1, 1, 0, 0, 1, 1, 1, 1, 0, 0, 1, 1, 0, 0, 0], //  7  forearms
    [0, 0, 0, 1, 1, 0, 0, 2, 2, 2, 2, 0, 0, 1, 1, 0, 0, 0], //  8  belt
    [0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0], //  9  lower torso
    [0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0], // 10  hips
    [0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0], // 11  upper legs
    [0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0], // 12  legs apart
    [0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0], // 13  legs wide
    [0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0], // 14  shins
    [0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0], // 15  ankles
    [0, 0, 0, 1, 1, 1, 0, 0, 0, 0, 0, 0, 1, 1, 1, 0, 0, 0], // 16  feet
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], // 17
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], // 18
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], // 19
];

// -- Stance B: throw / sweep (one arm extended, shifted weight) --
const STANCE_B: [[u8; W]; H] = [
    [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0], //  0  head top
    [0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0], //  1  head
    [0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0], //  2  head
    [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0], //  3  neck
    [0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0], //  4  shoulders
    [0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0], //  5  upper torso
    [0, 0, 1, 1, 1, 1, 0, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0], //  6  left arm extended
    [0, 1, 1, 0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0], //  7  left hand out
    [1, 1, 0, 0, 0, 0, 0, 2, 2, 2, 2, 0, 0, 0, 0, 0, 0, 0], //  8  belt
    [0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0], //  9  lower torso
    [0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0], // 10  hips
    [0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0], // 11  upper legs
    [0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0], // 12  legs bent
    [0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0], // 13  wide stance
    [0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0], // 14  shins
    [0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0], // 15  ankles
    [0, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 0, 0], // 16  feet
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], // 17
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], // 18
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], // 19
];

// -- Stance C: hip throw mid-motion (body rotated, leg sweeping) --
const STANCE_C: [[u8; W]; H] = [
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0], //  0  head top
    [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0], //  1  head
    [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0], //  2  head
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0], //  3  neck
    [0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0], //  4  shoulders
    [0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0], //  5  upper torso
    [0, 0, 0, 0, 0, 1, 1, 0, 1, 1, 1, 0, 1, 1, 0, 0, 0, 0], //  6  arms
    [0, 0, 0, 0, 1, 1, 0, 0, 1, 1, 1, 0, 0, 1, 1, 1, 0, 0], //  7  forearms + right extending
    [0, 0, 0, 0, 0, 0, 0, 0, 2, 2, 2, 0, 0, 0, 1, 1, 1, 0], //  8  belt + right hand
    [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0], //  9  lower torso
    [0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 1, 1, 0, 0, 0, 0, 0, 0], // 10  hips
    [0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0], // 11  upper legs
    [0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0], // 12  legs apart
    [0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0], // 13  sweep leg
    [0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0], // 14  sweeping
    [0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0], // 15
    [0, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 0, 0], // 16  feet
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], // 17
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], // 18
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], // 19
];

struct JujutsuColors {
    gi: egui::Color32,
    highlight: egui::Color32,
    shadow: egui::Color32,
    belt: egui::Color32,
    head: egui::Color32,
    enso: egui::Color32,
}

fn compute_colors(accent: egui::Color32, is_dark: bool) -> JujutsuColors {
    let [ar, ag, ab, _] = accent.to_array();

    let gi = if is_dark {
        egui::Color32::from_rgb(220, 218, 215)
    } else {
        egui::Color32::from_rgb(240, 238, 235)
    };
    let highlight = if is_dark {
        egui::Color32::from_rgb(245, 243, 240)
    } else {
        egui::Color32::from_rgb(255, 253, 250)
    };
    let shadow = if is_dark {
        egui::Color32::from_rgb(170, 168, 165)
    } else {
        egui::Color32::from_rgb(195, 193, 190)
    };
    let belt = accent;
    let head = egui::Color32::from_rgb(
        ar.saturating_sub(20),
        ag.saturating_sub(20),
        ab.saturating_sub(20),
    );
    let enso = egui::Color32::from_rgba_premultiplied(ar, ag, ab, if is_dark { 60 } else { 45 });

    JujutsuColors {
        gi,
        highlight,
        shadow,
        belt,
        head,
        enso,
    }
}

fn current_stance(t: f32) -> &'static [[u8; W]; H] {
    let phase = ((t * 0.8) % 3.0) as u32;
    match phase {
        0 => &STANCE_A,
        1 => &STANCE_B,
        _ => &STANCE_C,
    }
}

/// Total pixel dimensions of the widget at the given scale.
pub fn size(scale: f32) -> (f32, f32) {
    let px = PX * scale;
    (W as f32 * px, H as f32 * px)
}

/// Paint the jujutsu figure at `origin`.
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
    let stance = current_stance(t);

    // Rotating enso (incomplete circle) behind the figure.
    let center = origin + egui::vec2(W as f32 * px * 0.5, H as f32 * px * 0.45);
    let radius = 8.0 * px;
    let angle_offset = t * 0.6;
    let segments = 28;
    let gap = 6; // leave a gap for the enso's open stroke
    for i in 0..segments {
        if i >= segments - gap {
            continue;
        }
        let a0 = angle_offset + (i as f32 / segments as f32) * std::f32::consts::TAU;
        let a1 = angle_offset + ((i + 1) as f32 / segments as f32) * std::f32::consts::TAU;
        let p0 = center + egui::vec2(a0.cos() * radius, a0.sin() * radius);
        let p1 = center + egui::vec2(a1.cos() * radius, a1.sin() * radius);
        painter.line_segment([p0, p1], egui::Stroke::new(px * 0.8, colors.enso));
    }

    // Pixel figure
    #[allow(clippy::needless_range_loop)]
    for row in 0..H {
        for col in 0..W {
            let cell = stance[row][col];
            if cell == 0 {
                continue;
            }
            let above = if row > 0 { stance[row - 1][col] } else { 0 };
            let below = if row + 1 < H { stance[row + 1][col] } else { 0 };

            let color = if cell == 2 {
                colors.belt
            } else if row <= 2 {
                colors.head
            } else if above == 0 {
                colors.highlight
            } else if below == 0 {
                colors.shadow
            } else {
                colors.gi
            };

            let rect = egui::Rect::from_min_size(
                origin + egui::vec2(col as f32 * px, row as f32 * px),
                egui::vec2(px, px),
            );
            painter.rect_filled(rect, 0.0, color);
        }
    }

    // Floor / mat line
    let mat_color = egui::Color32::from_rgba_premultiplied(
        accent.r(),
        accent.g(),
        accent.b(),
        if is_dark { 80 } else { 60 },
    );
    let mat_row = 17;
    for col in 1..(W - 1) {
        let rect = egui::Rect::from_min_size(
            origin + egui::vec2(col as f32 * px, mat_row as f32 * px),
            egui::vec2(px, px),
        );
        painter.rect_filled(rect, 0.0, mat_color);
    }

    ctx.request_repaint_after(std::time::Duration::from_millis(100));
}
