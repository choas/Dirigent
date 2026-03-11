use eframe::egui;

use super::{DirigentApp, FONT_SCALE_HEADING, FONT_SCALE_SMALL};

/// Try to load a font from the system by name. Returns (bytes, ttc_index).
fn load_system_font(name: &str) -> Option<(Vec<u8>, u32)> {
    // Known font paths on macOS
    let candidates: &[(&str, u32)] = match name {
        "Menlo" => &[("/System/Library/Fonts/Menlo.ttc", 0)],
        "Monaco" => &[("/System/Library/Fonts/Monaco.ttf", 0)],
        "SF Mono" => &[("/System/Library/Fonts/SFNSMono.ttf", 0)],
        "Courier New" => &[
            ("/System/Library/Fonts/Supplemental/Courier New.ttf", 0),
            ("/Library/Fonts/Courier New.ttf", 0),
        ],
        _ => &[],
    };

    for &(path, index) in candidates {
        if let Ok(data) = std::fs::read(path) {
            return Some((data, index));
        }
    }

    // Try common font directories with various extensions
    let home = std::env::var("HOME").unwrap_or_default();
    let dirs = [
        "/System/Library/Fonts".to_string(),
        "/System/Library/Fonts/Supplemental".to_string(),
        "/Library/Fonts".to_string(),
        format!("{}/Library/Fonts", home),
    ];
    let exts = ["ttf", "ttc", "otf"];

    for dir in &dirs {
        for ext in &exts {
            let path = format!("{}/{}.{}", dir, name, ext);
            if let Ok(data) = std::fs::read(&path) {
                return Some((data, 0));
            }
        }
    }

    None
}

/// Returns a `RichText` using the dedicated icon font (SF Mono) at the given size.
pub(super) fn icon(text: &str, size: f32) -> egui::RichText {
    egui::RichText::new(text).font(egui::FontId::new(
        size,
        egui::FontFamily::Name("Icons".into()),
    ))
}

/// Returns a `RichText` using the dedicated icon font (SF Mono) at 75% of the given size.
pub(super) fn icon_small(text: &str, size: f32) -> egui::RichText {
    egui::RichText::new(text).font(egui::FontId::new(
        size * 0.75,
        egui::FontFamily::Name("Icons".into()),
    ))
}

impl DirigentApp {
    pub(super) fn apply_theme(&mut self, ctx: &egui::Context) {
        if !self.needs_theme_apply {
            return;
        }
        self.needs_theme_apply = false;
        ctx.set_visuals(self.settings.theme.visuals());
        self.semantic = self.settings.theme.semantic_colors();
        self.viewer.syntax_theme = if self.settings.theme.is_dark() {
            egui_extras::syntax_highlighting::CodeTheme::dark(self.settings.font_size)
        } else {
            egui_extras::syntax_highlighting::CodeTheme::light(self.settings.font_size)
        };

        let mut style = (*ctx.style()).clone();
        let font_family = &self.settings.font_family;
        let size = self.settings.font_size;

        // Load the user's chosen font from the system and register it with egui
        let mut font_def = egui::FontDefinitions::default();
        if let Some((font_bytes, index)) = load_system_font(font_family) {
            let mut font_data = egui::FontData::from_owned(font_bytes);
            font_data.index = index;
            font_def
                .font_data
                .insert(font_family.clone(), font_data.into());
            font_def
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .insert(0, font_family.clone());
            font_def
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, font_family.clone());
        }
        // Add symbol fallback fonts so icons render even when the chosen
        // code font lacks glyphs like ▶, ●, ↺, etc.
        // SF Mono has the best coverage for our icon characters, so it comes first.
        let symbol_fonts: &[(&str, &str, u32)] = &[
            (
                "DiriSymFallback_SFMono",
                "/System/Library/Fonts/SFNSMono.ttf",
                0,
            ),
            (
                "DiriSymFallback_Symbols",
                "/System/Library/Fonts/Apple Symbols.ttf",
                0,
            ),
            (
                "DiriSymFallback_Menlo",
                "/System/Library/Fonts/Menlo.ttc",
                0,
            ),
        ];
        for &(name, path, index) in symbol_fonts {
            if let Ok(data) = std::fs::read(path) {
                let mut fd = egui::FontData::from_owned(data);
                fd.index = index;
                font_def.font_data.insert(name.to_string(), fd.into());
                font_def
                    .families
                    .entry(egui::FontFamily::Monospace)
                    .or_default()
                    .push(name.to_string());
                font_def
                    .families
                    .entry(egui::FontFamily::Proportional)
                    .or_default()
                    .push(name.to_string());
                font_def
                    .families
                    .entry(egui::FontFamily::Name("Icons".into()))
                    .or_default()
                    .push(name.to_string());
            }
        }
        // Ensure the "Icons" family always exists so icon() / icon_small() never
        // panic.  When no symbol font was loaded, fall back to Monospace fonts.
        {
            let needs_fallback = font_def
                .families
                .get(&egui::FontFamily::Name("Icons".into()))
                .map_or(true, |v| v.is_empty());
            if needs_fallback {
                let mono = font_def
                    .families
                    .get(&egui::FontFamily::Monospace)
                    .cloned()
                    .unwrap_or_default();
                font_def
                    .families
                    .insert(egui::FontFamily::Name("Icons".into()), mono);
            }
        }
        ctx.set_fonts(font_def);

        // Scale all text styles based on the chosen font size
        style.text_styles.insert(
            egui::TextStyle::Small,
            egui::FontId::new(size * FONT_SCALE_SMALL, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Body,
            egui::FontId::new(size, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Monospace,
            egui::FontId::new(size, egui::FontFamily::Monospace),
        );
        style.text_styles.insert(
            egui::TextStyle::Button,
            egui::FontId::new(size, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Heading,
            egui::FontId::new(size * FONT_SCALE_HEADING, egui::FontFamily::Proportional),
        );
        ctx.set_style(style);
    }
}
