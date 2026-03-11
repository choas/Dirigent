use eframe::egui;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) enum ThemeChoice {
    // Dark themes
    Dark,
    Nord,
    Dracula,
    SolarizedDark,
    Monokai,
    GruvboxDark,
    TokyoNight,
    OneDark,
    CatppuccinMocha,
    EverforestDark,
    // Light themes
    Light,
    SolarizedLight,
    GruvboxLight,
    GitHubLight,
    CatppuccinLatte,
    EverforestLight,
    RosePineDawn,
    OneLight,
    NordLight,
    TokyoNightLight,
}

impl ThemeChoice {
    pub fn is_dark(&self) -> bool {
        matches!(
            self,
            ThemeChoice::Dark
                | ThemeChoice::Nord
                | ThemeChoice::Dracula
                | ThemeChoice::SolarizedDark
                | ThemeChoice::Monokai
                | ThemeChoice::GruvboxDark
                | ThemeChoice::TokyoNight
                | ThemeChoice::OneDark
                | ThemeChoice::CatppuccinMocha
                | ThemeChoice::EverforestDark
        )
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            ThemeChoice::Dark => "Dark",
            ThemeChoice::Nord => "Nord",
            ThemeChoice::Dracula => "Dracula",
            ThemeChoice::SolarizedDark => "Solarized Dark",
            ThemeChoice::Monokai => "Monokai",
            ThemeChoice::GruvboxDark => "Gruvbox Dark",
            ThemeChoice::TokyoNight => "Tokyo Night",
            ThemeChoice::OneDark => "One Dark",
            ThemeChoice::CatppuccinMocha => "Catppuccin Mocha",
            ThemeChoice::EverforestDark => "Everforest Dark",
            ThemeChoice::Light => "Light",
            ThemeChoice::SolarizedLight => "Solarized Light",
            ThemeChoice::GruvboxLight => "Gruvbox Light",
            ThemeChoice::GitHubLight => "GitHub Light",
            ThemeChoice::CatppuccinLatte => "Catppuccin Latte",
            ThemeChoice::EverforestLight => "Everforest Light",
            ThemeChoice::RosePineDawn => "Rosé Pine Dawn",
            ThemeChoice::OneLight => "One Light",
            ThemeChoice::NordLight => "Nord Light",
            ThemeChoice::TokyoNightLight => "Tokyo Night Light",
        }
    }

    pub fn all_variants() -> &'static [ThemeChoice] {
        &[
            // Dark
            ThemeChoice::Dark,
            ThemeChoice::Nord,
            ThemeChoice::Dracula,
            ThemeChoice::SolarizedDark,
            ThemeChoice::Monokai,
            ThemeChoice::GruvboxDark,
            ThemeChoice::TokyoNight,
            ThemeChoice::OneDark,
            ThemeChoice::CatppuccinMocha,
            ThemeChoice::EverforestDark,
            // Light
            ThemeChoice::Light,
            ThemeChoice::SolarizedLight,
            ThemeChoice::GruvboxLight,
            ThemeChoice::GitHubLight,
            ThemeChoice::CatppuccinLatte,
            ThemeChoice::EverforestLight,
            ThemeChoice::RosePineDawn,
            ThemeChoice::OneLight,
            ThemeChoice::NordLight,
            ThemeChoice::TokyoNightLight,
        ]
    }

    /// Returns custom egui Visuals for this theme.
    pub fn visuals(&self) -> egui::Visuals {
        self.palette().apply(self.is_dark())
    }

    fn palette(&self) -> ThemePalette {
        use ThemeChoice::*;
        match self {
            //                            panel_fill         window_fill        extreme_bg         faint_bg           text               selection          noninteractive     inactive           hovered            active             hyperlink
            Nord => palette!(
                [46, 52, 64],
                [59, 66, 82],
                [36, 41, 51],
                [59, 66, 82],
                [216, 222, 233],
                [94, 129, 172],
                [59, 66, 82],
                [67, 76, 94],
                [76, 86, 106],
                [94, 129, 172],
                [136, 192, 208]
            ),
            Dracula => palette!(
                [40, 42, 54],
                [68, 71, 90],
                [33, 34, 44],
                [68, 71, 90],
                [248, 248, 242],
                [68, 71, 90],
                [68, 71, 90],
                [55, 57, 73],
                [80, 83, 105],
                [189, 147, 249],
                [139, 233, 253]
            ),
            SolarizedDark => palette!(
                [0, 43, 54],
                [7, 54, 66],
                [0, 34, 43],
                [7, 54, 66],
                [131, 148, 150],
                [38, 139, 210],
                [7, 54, 66],
                [7, 54, 66],
                [88, 110, 117],
                [38, 139, 210],
                [38, 139, 210]
            ),
            Monokai => palette!(
                [39, 40, 34],
                [49, 50, 44],
                [30, 31, 26],
                [49, 50, 44],
                [248, 248, 242],
                [73, 72, 62],
                [49, 50, 44],
                [59, 60, 54],
                [73, 72, 62],
                [166, 226, 46],
                [102, 217, 239]
            ),
            GruvboxDark => palette!(
                [40, 40, 40],
                [50, 48, 47],
                [29, 32, 33],
                [50, 48, 47],
                [235, 219, 178],
                [69, 133, 136],
                [50, 48, 47],
                [60, 56, 54],
                [80, 73, 69],
                [152, 151, 26],
                [131, 165, 152]
            ),
            TokyoNight => palette!(
                [26, 27, 38],
                [36, 40, 59],
                [22, 22, 30],
                [36, 40, 59],
                [169, 177, 214],
                [42, 47, 78],
                [36, 40, 59],
                [41, 46, 73],
                [52, 59, 88],
                [122, 162, 247],
                [125, 207, 255]
            ),
            OneDark => palette!(
                [40, 44, 52],
                [50, 55, 65],
                [33, 37, 43],
                [50, 55, 65],
                [171, 178, 191],
                [62, 68, 81],
                [50, 55, 65],
                [55, 60, 72],
                [62, 68, 81],
                [97, 175, 239],
                [86, 182, 194]
            ),
            CatppuccinMocha => palette!(
                [30, 30, 46],
                [49, 50, 68],
                [17, 17, 27],
                [24, 24, 37],
                [205, 214, 244],
                [88, 91, 112],
                [49, 50, 68],
                [69, 71, 90],
                [88, 91, 112],
                [137, 180, 250],
                [116, 199, 236]
            ),
            EverforestDark => palette!(
                [47, 53, 57],
                [52, 58, 62],
                [39, 44, 48],
                [52, 58, 62],
                [211, 198, 170],
                [80, 88, 92],
                [52, 58, 62],
                [58, 65, 68],
                [68, 75, 79],
                [167, 192, 128],
                [127, 187, 179]
            ),
            SolarizedLight => palette!(
                [253, 246, 227],
                [238, 232, 213],
                [255, 250, 235],
                [238, 232, 213],
                [101, 123, 131],
                [38, 139, 210],
                [238, 232, 213],
                [238, 232, 213],
                [220, 215, 198],
                [38, 139, 210],
                [38, 139, 210]
            ),
            GruvboxLight => palette!(
                [251, 241, 199],
                [242, 229, 188],
                [255, 248, 210],
                [242, 229, 188],
                [60, 56, 54],
                [69, 133, 136],
                [242, 229, 188],
                [235, 219, 178],
                [213, 196, 161],
                [152, 151, 26],
                [7, 102, 120]
            ),
            GitHubLight => palette!(
                [255, 255, 255],
                [246, 248, 250],
                [255, 255, 255],
                [246, 248, 250],
                [36, 41, 46],
                [0, 92, 197],
                [246, 248, 250],
                [234, 238, 242],
                [220, 224, 228],
                [0, 92, 197],
                [3, 47, 98]
            ),
            CatppuccinLatte => palette!(
                [239, 241, 245],
                [204, 208, 218],
                [220, 224, 232],
                [230, 233, 239],
                [76, 79, 105],
                [172, 176, 190],
                [204, 208, 218],
                [188, 192, 204],
                [172, 176, 190],
                [30, 102, 245],
                [32, 159, 181]
            ),
            EverforestLight => palette!(
                [253, 246, 227],
                [242, 237, 220],
                [255, 252, 238],
                [242, 237, 220],
                [92, 99, 78],
                [160, 188, 132],
                [242, 237, 220],
                [230, 225, 208],
                [218, 213, 196],
                [141, 165, 104],
                [53, 162, 147]
            ),
            RosePineDawn => palette!(
                [250, 244, 237],
                [255, 250, 243],
                [242, 233, 222],
                [255, 250, 243],
                [87, 82, 121],
                [215, 130, 126],
                [255, 250, 243],
                [242, 233, 222],
                [232, 222, 210],
                [87, 82, 121],
                [40, 105, 131]
            ),
            OneLight => palette!(
                [250, 250, 250],
                [240, 240, 240],
                [255, 255, 255],
                [240, 240, 240],
                [56, 58, 66],
                [198, 216, 240],
                [240, 240, 240],
                [232, 232, 232],
                [218, 218, 218],
                [64, 120, 242],
                [1, 132, 188]
            ),
            NordLight => palette!(
                [236, 239, 244],
                [229, 233, 240],
                [242, 245, 250],
                [229, 233, 240],
                [59, 66, 82],
                [136, 192, 208],
                [229, 233, 240],
                [216, 222, 233],
                [208, 214, 225],
                [94, 129, 172],
                [94, 129, 172]
            ),
            TokyoNightLight => palette!(
                [213, 214, 219],
                [224, 225, 228],
                [235, 236, 240],
                [224, 225, 228],
                [52, 54, 86],
                [180, 182, 200],
                [224, 225, 228],
                [210, 211, 216],
                [198, 199, 206],
                [52, 84, 223],
                [118, 105, 199]
            ),
            //                            panel_fill         window_fill        extreme_bg         faint_bg           text               selection          noninteractive     inactive           hovered            active             hyperlink
            Dark => palette!(
                [32, 33, 38],
                [42, 44, 52],
                [22, 23, 26],
                [38, 40, 46],
                [210, 214, 222],
                [42, 62, 110],
                [42, 44, 52],
                [48, 50, 60],
                [58, 62, 74],
                [100, 180, 255],
                [100, 180, 255]
            ),
            Light => palette!(
                [244, 245, 248],
                [252, 252, 255],
                [255, 255, 255],
                [236, 238, 243],
                [30, 32, 42],
                [178, 210, 250],
                [236, 238, 243],
                [224, 226, 234],
                [212, 216, 226],
                [0, 100, 220],
                [0, 100, 220]
            ),
        }
    }
}

macro_rules! palette {
    ([$pr:expr,$pg:expr,$pb:expr], [$wr:expr,$wg:expr,$wb:expr], [$er:expr,$eg:expr,$eb:expr], [$fr:expr,$fg:expr,$fb:expr], [$tr:expr,$tg:expr,$tb:expr], [$sr:expr,$sg:expr,$sb:expr], [$nr:expr,$ng:expr,$nb:expr], [$ir:expr,$ig:expr,$ib:expr], [$hr:expr,$hg:expr,$hb:expr], [$ar:expr,$ag:expr,$ab:expr], [$lr:expr,$lg:expr,$lb:expr]) => {
        ThemePalette {
            panel_fill: egui::Color32::from_rgb($pr, $pg, $pb),
            window_fill: egui::Color32::from_rgb($wr, $wg, $wb),
            extreme_bg: egui::Color32::from_rgb($er, $eg, $eb),
            faint_bg: egui::Color32::from_rgb($fr, $fg, $fb),
            text: egui::Color32::from_rgb($tr, $tg, $tb),
            selection: egui::Color32::from_rgb($sr, $sg, $sb),
            noninteractive: egui::Color32::from_rgb($nr, $ng, $nb),
            inactive: egui::Color32::from_rgb($ir, $ig, $ib),
            hovered: egui::Color32::from_rgb($hr, $hg, $hb),
            active: egui::Color32::from_rgb($ar, $ag, $ab),
            hyperlink: egui::Color32::from_rgb($lr, $lg, $lb),
        }
    };
}
use palette;

struct ThemePalette {
    panel_fill: egui::Color32,
    window_fill: egui::Color32,
    extreme_bg: egui::Color32,
    faint_bg: egui::Color32,
    text: egui::Color32,
    selection: egui::Color32,
    noninteractive: egui::Color32,
    inactive: egui::Color32,
    hovered: egui::Color32,
    active: egui::Color32,
    hyperlink: egui::Color32,
}

/// Apply generous, Apple-style corner rounding and subtle depth to all visual elements.
/// 4px for small widgets, 8px for menus, 12px for dialog windows.
fn apply_rounding_and_depth(v: &mut egui::Visuals, dark: bool) {
    let r = egui::Rounding::same;
    v.window_rounding = r(12.0);
    v.menu_rounding = r(8.0);
    v.widgets.noninteractive.rounding = r(6.0);
    v.widgets.inactive.rounding = r(6.0);
    v.widgets.hovered.rounding = r(8.0);
    v.widgets.active.rounding = r(6.0);
    v.widgets.open.rounding = r(6.0);

    // Subtle drop shadows for floating windows and popups
    let shadow_alpha = if dark { 100 } else { 40 };
    v.window_shadow = egui::epaint::Shadow {
        offset: egui::vec2(0.0, 4.0),
        blur: 16.0,
        spread: 2.0,
        color: egui::Color32::from_black_alpha(shadow_alpha),
    };
    v.popup_shadow = egui::epaint::Shadow {
        offset: egui::vec2(0.0, 2.0),
        blur: 8.0,
        spread: 1.0,
        color: egui::Color32::from_black_alpha(shadow_alpha),
    };
}

/// Apply interactive visual polish: press-in effect, accent-colored focus ring.
fn apply_interactive_visuals(v: &mut egui::Visuals, accent: egui::Color32) {
    // Press-in: shrink the painted background when pressed (hover expands +1, press shrinks −0.5)
    v.widgets.active.expansion = -0.5;

    // Focus ring: accent-colored stroke on active/focused widgets
    v.selection.stroke = egui::Stroke::new(1.5, accent);
    let [r, g, b, _] = accent.to_array();
    v.widgets.hovered.bg_stroke =
        egui::Stroke::new(1.0, egui::Color32::from_rgba_premultiplied(r, g, b, 80));
    v.widgets.active.bg_stroke = egui::Stroke::new(1.5, accent);
}

impl ThemePalette {
    fn apply(self, dark: bool) -> egui::Visuals {
        let mut v = if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        v.panel_fill = self.panel_fill;
        v.window_fill = self.window_fill;
        v.extreme_bg_color = self.extreme_bg;
        v.faint_bg_color = self.faint_bg;
        v.override_text_color = Some(self.text);
        v.selection.bg_fill = self.selection;
        v.widgets.noninteractive.bg_fill = self.noninteractive;
        v.widgets.inactive.bg_fill = self.inactive;
        v.widgets.hovered.bg_fill = self.hovered;
        v.widgets.active.bg_fill = self.active;
        v.hyperlink_color = self.hyperlink;
        apply_rounding_and_depth(&mut v, dark);
        apply_interactive_visuals(&mut v, self.hyperlink);
        v
    }
}

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
    is_dark: bool,
}

impl SemanticColors {
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
            egui::Color32::from_rgba_premultiplied(180, 180, 0, 30)
        }
    }

    pub fn search_highlight_bg(&self) -> egui::Color32 {
        if self.is_dark {
            egui::Color32::from_rgb(180, 140, 0)
        } else {
            egui::Color32::from_rgb(200, 160, 0)
        }
    }

    pub fn code_search_current(&self) -> egui::Color32 {
        if self.is_dark {
            egui::Color32::from_rgba_premultiplied(200, 120, 0, 60)
        } else {
            egui::Color32::from_rgba_premultiplied(255, 180, 0, 80)
        }
    }

    pub fn code_search_match(&self) -> egui::Color32 {
        if self.is_dark {
            egui::Color32::from_rgba_premultiplied(180, 160, 0, 35)
        } else {
            egui::Color32::from_rgba_premultiplied(220, 200, 0, 40)
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
            .rounding(egui::Rounding::same(8.0))
            .shadow(egui::epaint::Shadow {
                offset: egui::vec2(0.0, 4.0),
                blur: 16.0,
                spread: 4.0,
                color: self.dialog_shadow(),
            })
            .inner_margin(egui::Margin::same(16.0))
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
            .inner_margin(egui::Margin::same(24.0))
            .shadow(egui::epaint::Shadow {
                offset: egui::vec2(0.0, 8.0),
                blur: 24.0,
                spread: 8.0,
                color: self.dialog_shadow(),
            })
            .rounding(egui::Rounding::same(12.0))
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
        egui::Frame::none()
            .inner_margin(10.0)
            .fill(self.card_surface())
            .rounding(8.0)
            .shadow(egui::epaint::Shadow {
                offset: egui::vec2(0.0, 2.0),
                blur: 6.0,
                spread: 0.0,
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

impl ThemeChoice {
    pub fn semantic_colors(&self) -> SemanticColors {
        use ThemeChoice::*;
        let dark = self.is_dark();

        // Accent color follows each theme's identity (hyperlink) color
        let accent = match self {
            Dark => egui::Color32::from_rgb(100, 180, 255),
            Nord => egui::Color32::from_rgb(136, 192, 208),
            Dracula => egui::Color32::from_rgb(139, 233, 253),
            SolarizedDark => egui::Color32::from_rgb(38, 139, 210),
            Monokai => egui::Color32::from_rgb(102, 217, 239),
            GruvboxDark => egui::Color32::from_rgb(131, 165, 152),
            TokyoNight => egui::Color32::from_rgb(125, 207, 255),
            OneDark => egui::Color32::from_rgb(86, 182, 194),
            CatppuccinMocha => egui::Color32::from_rgb(116, 199, 236),
            EverforestDark => egui::Color32::from_rgb(127, 187, 179),
            Light => egui::Color32::from_rgb(0, 100, 220),
            SolarizedLight => egui::Color32::from_rgb(38, 139, 210),
            GruvboxLight => egui::Color32::from_rgb(7, 102, 120),
            GitHubLight => egui::Color32::from_rgb(3, 47, 98),
            CatppuccinLatte => egui::Color32::from_rgb(32, 159, 181),
            EverforestLight => egui::Color32::from_rgb(53, 162, 147),
            RosePineDawn => egui::Color32::from_rgb(40, 105, 131),
            OneLight => egui::Color32::from_rgb(1, 132, 188),
            NordLight => egui::Color32::from_rgb(94, 129, 172),
            TokyoNightLight => egui::Color32::from_rgb(118, 105, 199),
        };

        if dark {
            SemanticColors {
                accent,
                success: egui::Color32::from_rgb(80, 190, 110),
                warning: egui::Color32::from_rgb(200, 165, 60),
                danger: egui::Color32::from_rgb(210, 95, 95),
                secondary_text: egui::Color32::from_gray(160),
                tertiary_text: egui::Color32::from_gray(120),
                separator: egui::Color32::from_gray(60),
                badge_text: egui::Color32::from_gray(220),
                is_dark: true,
            }
        } else {
            SemanticColors {
                accent,
                success: egui::Color32::from_rgb(30, 140, 30),
                warning: egui::Color32::from_rgb(160, 110, 0),
                danger: egui::Color32::from_rgb(200, 50, 50),
                secondary_text: egui::Color32::from_gray(100),
                tertiary_text: egui::Color32::from_gray(150),
                separator: egui::Color32::from_gray(200),
                badge_text: egui::Color32::from_gray(255),
                is_dark: false,
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) enum CliProvider {
    Claude,
    OpenCode,
}

impl CliProvider {
    pub fn display_name(&self) -> &'static str {
        match self {
            CliProvider::Claude => "Claude",
            CliProvider::OpenCode => "OpenCode",
        }
    }

    pub fn all() -> &'static [CliProvider] {
        &[CliProvider::Claude, CliProvider::OpenCode]
    }
}

impl Default for CliProvider {
    fn default() -> Self {
        CliProvider::Claude
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) enum SourceKind {
    GitHubIssues,
    Notion,
    Mcp,
    Custom,
}

impl SourceKind {
    pub fn display_name(&self) -> &'static str {
        match self {
            SourceKind::GitHubIssues => "GitHub Issues",
            SourceKind::Notion => "Notion",
            SourceKind::Mcp => "MCP",
            SourceKind::Custom => "Custom",
        }
    }

    pub fn all() -> &'static [SourceKind] {
        &[
            SourceKind::GitHubIssues,
            SourceKind::Notion,
            SourceKind::Mcp,
            SourceKind::Custom,
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SourceConfig {
    pub name: String,
    pub kind: SourceKind,
    pub label: String,
    pub poll_interval_secs: u64,
    pub enabled: bool,
    #[serde(default)]
    pub filter: String,
    #[serde(default)]
    pub command: String,
}

impl Default for SourceConfig {
    fn default() -> Self {
        SourceConfig {
            name: "New Source".to_string(),
            kind: SourceKind::GitHubIssues,
            label: "github".to_string(),
            poll_interval_secs: 300,
            enabled: true,
            filter: String::new(),
            command: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Play {
    pub name: String,
    pub prompt: String,
}

pub(crate) fn default_playbook() -> Vec<Play> {
    vec![
        Play {
            name: "Update README".into(),
            prompt: "Review the project and update README.md to accurately reflect the current state: features, setup instructions, and usage.".into(),
        },
        Play {
            name: "Verify architecture".into(),
            prompt: "Analyze the project architecture. Check for structural issues, circular dependencies, inconsistent patterns. Report findings without making changes.".into(),
        },
        Play {
            name: "Verify last 5 commits".into(),
            prompt: "Review the last 5 git commits. Check for bugs, incomplete changes, or inconsistencies. Report findings without making changes.".into(),
        },
        Play {
            name: "Create release".into(),
            prompt: "Prepare a release: update version numbers, ensure CHANGELOG is current, verify tests pass, create a release commit.".into(),
        },
        Play {
            name: "Security audit".into(),
            prompt: "Check for hardcoded secrets, insecure dependencies, injection vulnerabilities, unsafe code patterns. Report findings.".into(),
        },
        Play {
            name: "Check dead code".into(),
            prompt: "Find unused functions, unreachable branches, unused imports, stale modules. Report findings without removing anything.".into(),
        },
        Play {
            name: "Add tests".into(),
            prompt: "Identify untested code paths and write comprehensive tests for the most critical and least covered areas.".into(),
        },
        Play {
            name: "Fix all warnings".into(),
            prompt: "Run `cargo check`, collect all warnings, and fix every one of them.".into(),
        },
        Play {
            name: "Commit changes".into(),
            prompt: "Commit all current changes. Open the SQLite database (find the .db file in the repo) and query the cues table for rows with status 'done' or 'review'. Use their titles to write a meaningful commit message summarizing what was accomplished. Then UPDATE any cues with status='review' to status='done'. Finally, stage all changes with git and create the commit.".into(),
        },
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Settings {
    pub theme: ThemeChoice,
    pub cli_provider: CliProvider,
    pub claude_model: String,
    #[serde(default)]
    pub claude_cli_path: String,
    #[serde(default)]
    pub claude_extra_args: String,
    pub opencode_model: String,
    #[serde(default)]
    pub opencode_cli_path: String,
    #[serde(default)]
    pub opencode_extra_args: String,
    pub recent_repos: Vec<String>,
    #[serde(default = "default_true")]
    pub notify_sound: bool,
    #[serde(default = "default_true")]
    pub notify_popup: bool,
    #[serde(default = "default_font_family")]
    pub font_family: String,
    #[serde(default = "default_font_size")]
    pub font_size: f32,
    #[serde(default)]
    pub sources: Vec<SourceConfig>,
    #[serde(default = "default_playbook")]
    pub playbook: Vec<Play>,
}

fn default_true() -> bool {
    true
}

fn default_font_family() -> String {
    "Menlo".to_string()
}

fn default_font_size() -> f32 {
    13.0
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: ThemeChoice::Dark,
            cli_provider: CliProvider::default(),
            claude_model: "claude-opus-4-6".to_string(),
            claude_cli_path: String::new(),
            claude_extra_args: String::new(),
            opencode_model: "openai/o1".to_string(),
            opencode_cli_path: String::new(),
            opencode_extra_args: String::new(),
            recent_repos: Vec::new(),
            notify_sound: true,
            notify_popup: true,
            font_family: default_font_family(),
            font_size: default_font_size(),
            sources: Vec::new(),
            playbook: default_playbook(),
        }
    }
}

pub(crate) fn load_settings(project_root: &Path) -> Settings {
    let path = project_root.join(".Dirigent").join("settings.json");
    let mut settings = match std::fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => Settings::default(),
    };
    // Append any new default plays that aren't already in the user's playbook
    for default_play in default_playbook() {
        if !settings
            .playbook
            .iter()
            .any(|p| p.name == default_play.name)
        {
            settings.playbook.push(default_play);
        }
    }
    settings
}

pub(crate) fn save_settings(project_root: &Path, settings: &Settings) {
    let dir = project_root.join(".Dirigent");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("settings.json");
    if let Ok(json) = serde_json::to_string_pretty(settings) {
        let _ = std::fs::write(path, json);
    }
}

pub(crate) fn add_recent_repo(settings: &mut Settings, path: &str) {
    settings.recent_repos.retain(|p| p != path);
    settings.recent_repos.insert(0, path.to_string());
    settings.recent_repos.truncate(10);
}
