use eframe::egui;

use super::providers::CliProvider;

/// Semantic colors that adapt to the current theme, replacing hardcoded RGB values.
/// Every UI element should use these instead of raw Color32 literals.
#[derive(Clone, Copy)]
pub(crate) struct SemanticColors {
    pub accent: egui::Color32,
    pub success: egui::Color32,
    pub warning: egui::Color32,
    pub danger: egui::Color32,
    pub secondary_text: egui::Color32,
    pub tertiary_text: egui::Color32,
    pub separator: egui::Color32,
    pub badge_text: egui::Color32,
    pub(super) is_dark: bool,
}

impl SemanticColors {
    pub fn is_dark(&self) -> bool {
        self.is_dark
    }

    pub fn selection_bg(&self) -> egui::Color32 {
        if self.is_dark {
            egui::Color32::from_rgba_premultiplied(60, 60, 120, 80)
        } else {
            egui::Color32::from_rgba_premultiplied(60, 100, 180, 50)
        }
    }

    pub fn addition_bg(&self) -> egui::Color32 {
        if self.is_dark {
            egui::Color32::from_rgba_premultiplied(25, 80, 35, 70)
        } else {
            egui::Color32::from_rgba_premultiplied(30, 120, 30, 35)
        }
    }

    pub fn deletion_bg(&self) -> egui::Color32 {
        if self.is_dark {
            egui::Color32::from_rgba_premultiplied(80, 25, 25, 70)
        } else {
            egui::Color32::from_rgba_premultiplied(120, 30, 30, 35)
        }
    }

    pub fn search_current_bg(&self) -> egui::Color32 {
        if self.is_dark {
            egui::Color32::from_rgba_premultiplied(100, 80, 0, 80)
        } else {
            egui::Color32::from_rgba_premultiplied(180, 150, 0, 60)
        }
    }

    pub fn search_match_bg(&self) -> egui::Color32 {
        if self.is_dark {
            egui::Color32::from_rgba_premultiplied(80, 80, 0, 40)
        } else {
            egui::Color32::from_rgba_premultiplied(160, 130, 20, 28)
        }
    }

    pub fn search_highlight_bg(&self) -> egui::Color32 {
        if self.is_dark {
            egui::Color32::from_rgb(180, 140, 0)
        } else {
            egui::Color32::from_rgba_premultiplied(180, 120, 40, 90)
        }
    }

    pub fn code_search_current(&self) -> egui::Color32 {
        if self.is_dark {
            egui::Color32::from_rgba_premultiplied(200, 120, 0, 60)
        } else {
            egui::Color32::from_rgba_premultiplied(200, 140, 40, 65)
        }
    }

    pub fn code_search_match(&self) -> egui::Color32 {
        if self.is_dark {
            egui::Color32::from_rgba_premultiplied(180, 160, 0, 35)
        } else {
            egui::Color32::from_rgba_premultiplied(180, 150, 40, 30)
        }
    }

    pub fn diff_header(&self) -> egui::Color32 {
        if self.is_dark {
            egui::Color32::from_rgb(150, 150, 220)
        } else {
            egui::Color32::from_rgb(60, 60, 140)
        }
    }

    pub fn claude_color(&self) -> egui::Color32 {
        if self.is_dark {
            egui::Color32::from_rgb(200, 160, 100)
        } else {
            egui::Color32::from_rgb(140, 100, 40)
        }
    }

    pub fn opencode_color(&self) -> egui::Color32 {
        if self.is_dark {
            egui::Color32::from_rgb(100, 200, 150)
        } else {
            egui::Color32::from_rgb(40, 120, 80)
        }
    }

    pub fn provider_color(&self, provider: &CliProvider) -> egui::Color32 {
        match provider {
            CliProvider::Claude => self.claude_color(),
            CliProvider::OpenCode => self.opencode_color(),
        }
    }

    pub fn muted_text(&self) -> egui::Color32 {
        if self.is_dark {
            egui::Color32::from_rgb(140, 140, 140)
        } else {
            egui::Color32::from_rgb(120, 120, 120)
        }
    }

    pub fn global_label(&self) -> egui::Color32 {
        if self.is_dark {
            egui::Color32::from_rgb(180, 140, 255)
        } else {
            egui::Color32::from_rgb(100, 60, 180)
        }
    }

    pub fn status_message_with_alpha(&self, alpha: f32) -> egui::Color32 {
        if self.is_dark {
            egui::Color32::from_rgba_unmultiplied(180, 180, 140, (alpha * 255.0) as u8)
        } else {
            egui::Color32::from_rgba_unmultiplied(100, 100, 40, (alpha * 255.0) as u8)
        }
    }

    pub fn modal_overlay(&self) -> egui::Color32 {
        egui::Color32::from_black_alpha(77) // ~30% opacity
    }

    /// Slightly elevated surface for the prompt field area.
    pub fn prompt_surface(&self) -> egui::Color32 {
        if self.is_dark {
            egui::Color32::from_white_alpha(8)
        } else {
            egui::Color32::from_black_alpha(8)
        }
    }

    /// Top border color for the prompt field.
    pub fn prompt_border(&self) -> egui::Color32 {
        if self.is_dark {
            egui::Color32::from_white_alpha(20)
        } else {
            egui::Color32::from_black_alpha(15)
        }
    }

    /// Slightly elevated surface color for dialog windows.
    pub fn dialog_surface(&self) -> egui::Color32 {
        if self.is_dark {
            egui::Color32::from_gray(38)
        } else {
            egui::Color32::from_gray(248)
        }
    }

    /// Shadow color for dialog windows.
    pub fn dialog_shadow(&self) -> egui::Color32 {
        if self.is_dark {
            egui::Color32::from_black_alpha(120)
        } else {
            egui::Color32::from_black_alpha(50)
        }
    }

    /// Frame for modal dialog windows — elevated surface, shadow, rounded corners.
    pub fn dialog_frame(&self) -> egui::Frame {
        egui::Frame::window(&egui::Style::default())
            .fill(self.dialog_surface())
            .corner_radius(egui::CornerRadius::same(8))
            .shadow(egui::epaint::Shadow {
                offset: [0, 4],
                blur: 16,
                spread: 4,
                color: self.dialog_shadow(),
            })
            .inner_margin(egui::Margin::same(16))
            .stroke(egui::Stroke::new(
                1.0,
                if self.is_dark {
                    egui::Color32::from_white_alpha(20)
                } else {
                    egui::Color32::from_black_alpha(15)
                },
            ))
    }

    /// Special frame for the About dialog — larger padding and more prominent shadow.
    pub fn about_dialog_frame(&self) -> egui::Frame {
        self.dialog_frame()
            .inner_margin(egui::Margin::same(24))
            .shadow(egui::epaint::Shadow {
                offset: [0, 8],
                blur: 24,
                spread: 8,
                color: self.dialog_shadow(),
            })
            .corner_radius(egui::CornerRadius::same(12))
    }

    /// Slightly elevated card surface color.
    fn card_surface(&self) -> egui::Color32 {
        if self.is_dark {
            egui::Color32::from_white_alpha(12)
        } else {
            egui::Color32::from_black_alpha(6)
        }
    }

    /// Frame for inline cards (cue cards, source configs, playbook items) —
    /// subtle shadow and elevated fill instead of a flat border.
    pub fn card_frame(&self) -> egui::Frame {
        let shadow_alpha = if self.is_dark { 70 } else { 25 };
        egui::Frame::NONE
            .inner_margin(10.0)
            .fill(self.card_surface())
            .corner_radius(8)
            .shadow(egui::epaint::Shadow {
                offset: [0, 2],
                blur: 6,
                spread: 0,
                color: egui::Color32::from_black_alpha(shadow_alpha),
            })
            .stroke(egui::Stroke::new(
                0.5,
                if self.is_dark {
                    egui::Color32::from_white_alpha(15)
                } else {
                    egui::Color32::from_black_alpha(10)
                },
            ))
    }

    /// Contrasting text color for use on accent-colored backgrounds.
    pub fn accent_text(&self) -> egui::Color32 {
        let [r, g, b, _] = self.accent.to_array();
        let luminance = r as u16 + g as u16 + b as u16;
        if luminance > 380 {
            egui::Color32::from_gray(30)
        } else {
            egui::Color32::WHITE
        }
    }
}
