use eframe::egui;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::agents::{default_agents, AgentConfig};

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
    let r = egui::CornerRadius::same;
    v.window_corner_radius = r(12);
    v.menu_corner_radius = r(8);
    v.widgets.noninteractive.corner_radius = r(6);
    v.widgets.inactive.corner_radius = r(6);
    v.widgets.hovered.corner_radius = r(8);
    v.widgets.active.corner_radius = r(6);
    v.widgets.open.corner_radius = r(6);

    // Subtle drop shadows for floating windows and popups
    let shadow_alpha = if dark { 100 } else { 40 };
    v.window_shadow = egui::epaint::Shadow {
        offset: [0, 4],
        blur: 16,
        spread: 2,
        color: egui::Color32::from_black_alpha(shadow_alpha),
    };
    v.popup_shadow = egui::epaint::Shadow {
        offset: [0, 2],
        blur: 8,
        spread: 1,
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

impl ThemeChoice {
    pub fn semantic_colors(&self) -> SemanticColors {
        use ThemeChoice::*;
        let dark = self.is_dark();

        // Each theme gets a distinctive accent color true to its palette
        let accent = match self {
            Dark => egui::Color32::from_rgb(100, 180, 255), // soft blue
            Nord => egui::Color32::from_rgb(163, 190, 140), // nord green (nord14)
            Dracula => egui::Color32::from_rgb(189, 147, 249), // dracula purple
            SolarizedDark => egui::Color32::from_rgb(181, 137, 0), // solarized yellow
            Monokai => egui::Color32::from_rgb(249, 38, 114), // monokai pink
            GruvboxDark => egui::Color32::from_rgb(215, 153, 33), // gruvbox yellow
            TokyoNight => egui::Color32::from_rgb(187, 154, 247), // tokyo night purple
            OneDark => egui::Color32::from_rgb(224, 108, 117), // one dark red
            CatppuccinMocha => egui::Color32::from_rgb(203, 166, 247), // catppuccin mauve
            EverforestDark => egui::Color32::from_rgb(167, 192, 128), // everforest green
            Light => egui::Color32::from_rgb(0, 100, 220),  // classic blue
            SolarizedLight => egui::Color32::from_rgb(133, 153, 0), // solarized green
            GruvboxLight => egui::Color32::from_rgb(175, 58, 3), // gruvbox orange
            GitHubLight => egui::Color32::from_rgb(9, 105, 218), // github blue
            CatppuccinLatte => egui::Color32::from_rgb(136, 57, 239), // catppuccin mauve
            EverforestLight => egui::Color32::from_rgb(93, 137, 98), // everforest green
            RosePineDawn => egui::Color32::from_rgb(215, 130, 126), // rose pine rose
            OneLight => egui::Color32::from_rgb(166, 38, 164), // one light purple
            NordLight => egui::Color32::from_rgb(94, 129, 172), // nord blue (nord10)
            TokyoNightLight => egui::Color32::from_rgb(118, 105, 199), // tokyo purple
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
                success: egui::Color32::from_rgb(10, 100, 10),
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
    Slack,
    SonarQube,
    Notion,
    Mcp,
    Custom,
}

impl SourceKind {
    pub fn display_name(&self) -> &'static str {
        match self {
            SourceKind::GitHubIssues => "GitHub Issues",
            SourceKind::Slack => "Slack",
            SourceKind::SonarQube => "SonarQube",
            SourceKind::Notion => "Notion",
            SourceKind::Mcp => "MCP",
            SourceKind::Custom => "Custom",
        }
    }

    pub fn default_label(&self) -> &'static str {
        match self {
            SourceKind::GitHubIssues => "github",
            SourceKind::Slack => "slack",
            SourceKind::SonarQube => "sonar",
            SourceKind::Notion => "notion",
            SourceKind::Mcp => "mcp",
            SourceKind::Custom => "custom",
        }
    }

    pub fn all() -> &'static [SourceKind] {
        &[
            SourceKind::GitHubIssues,
            SourceKind::Slack,
            SourceKind::SonarQube,
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
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub channel: String,
    #[serde(default)]
    pub host_url: String,
    #[serde(default)]
    pub project_key: String,
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
            token: String::new(),
            channel: String::new(),
            host_url: String::new(),
            project_key: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Play {
    pub name: String,
    pub prompt: String,
}

/// A parsed template variable from a play prompt, e.g. `{LICENSE:MIT,Apache 2.0,ISC}`.
#[derive(Debug, Clone)]
pub(crate) struct PlayVariable {
    /// Variable name (e.g. "LICENSE").
    pub name: String,
    /// Predefined options (may be empty for free-text variables).
    pub options: Vec<String>,
    /// The full matched token including braces, for substitution.
    pub token: String,
}

/// Parse template variables from a play prompt.
/// Syntax: `{VAR_NAME:option1,option2,...}` or `{VAR_NAME}` for free-text.
pub(crate) fn parse_play_variables(prompt: &str) -> Vec<PlayVariable> {
    let mut vars = Vec::new();
    let mut seen_tokens = std::collections::HashSet::new();
    let mut rest = prompt;
    while let Some(start) = rest.find('{') {
        if let Some(end) = rest[start..].find('}') {
            let token = &rest[start..start + end + 1];
            let inner = &rest[start + 1..start + end];
            if (!inner.is_empty() && !inner.contains(' ')) || inner.contains(':') {
                let (name, options) = if let Some(colon) = inner.find(':') {
                    let name = inner[..colon].to_string();
                    let opts: Vec<String> = inner[colon + 1..]
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    (name, opts)
                } else {
                    (inner.to_string(), Vec::new())
                };
                if !name.is_empty() && seen_tokens.insert(token.to_string()) {
                    vars.push(PlayVariable {
                        name,
                        options,
                        token: token.to_string(),
                    });
                }
            }
            rest = &rest[start + end + 1..];
        } else {
            break;
        }
    }
    vars
}

/// Substitute resolved variables back into the prompt template.
pub(crate) fn substitute_play_variables(
    prompt: &str,
    resolved: &[(String, String)], // (token, value)
) -> String {
    let mut result = prompt.to_string();
    for (token, value) in resolved {
        result = result.replace(token, value);
    }
    result
}

/// A command mode that can be triggered by prefixing a cue with `[command_name]`.
/// Commands wrap the cue text with additional prompt instructions and can
/// override the pre/post run scripts for that particular execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CueCommand {
    /// Short identifier used in `[name]` prefix (e.g. "plan", "test").
    pub name: String,
    /// Prompt template. `{task}` is replaced with the user's cue text.
    pub prompt: String,
    /// Shell command to run before the CLI invocation (overrides provider default).
    #[serde(default)]
    pub pre_agent: String,
    /// Shell command to run after the CLI invocation (overrides provider default).
    #[serde(default)]
    pub post_agent: String,
}

pub(crate) fn default_commands() -> Vec<CueCommand> {
    vec![
        CueCommand {
            name: "plan".into(),
            prompt: "Analyze the following task and create a detailed implementation plan. Identify the files that need to change, the approach, edge cases, and risks. Do NOT make any code changes — only output the plan.\n\nTask: {task}".into(),
            pre_agent: String::new(),
            post_agent: String::new(),
        },
        CueCommand {
            name: "test".into(),
            prompt: "Write comprehensive tests for the following. Cover happy paths, edge cases, and error conditions.\n\nWhat to test: {task}".into(),
            pre_agent: String::new(),
            post_agent: String::new(),
        },
        CueCommand {
            name: "refactor".into(),
            prompt: "Refactor the following for clarity, maintainability, and idiomatic style. Preserve all existing behavior — do not change functionality.\n\nWhat to refactor: {task}".into(),
            pre_agent: String::new(),
            post_agent: String::new(),
        },
        CueCommand {
            name: "review".into(),
            prompt: "Review the following code or area for bugs, security issues, performance problems, and style concerns. Report findings with file paths and line numbers. Do NOT make any code changes.\n\nWhat to review: {task}".into(),
            pre_agent: String::new(),
            post_agent: String::new(),
        },
        CueCommand {
            name: "fix".into(),
            prompt: "Fix the following bug or issue. Identify the root cause, apply the minimal correct fix, and verify nothing else breaks.\n\nIssue: {task}".into(),
            pre_agent: String::new(),
            post_agent: String::new(),
        },
        CueCommand {
            name: "docs".into(),
            prompt: "Write or update documentation for the following. Include clear explanations, examples where helpful, and keep it concise.\n\nWhat to document: {task}".into(),
            pre_agent: String::new(),
            post_agent: String::new(),
        },
        CueCommand {
            name: "explain".into(),
            prompt: "Explain how the following works in detail. Walk through the control flow, data structures, and key decisions. Do NOT make any code changes.\n\nWhat to explain: {task}".into(),
            pre_agent: String::new(),
            post_agent: String::new(),
        },
        CueCommand {
            name: "optimize".into(),
            prompt: "Optimize the following for performance. Profile or reason about bottlenecks, then apply targeted improvements. Preserve correctness.\n\nWhat to optimize: {task}".into(),
            pre_agent: String::new(),
            post_agent: String::new(),
        },
    ]
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
            prompt: "Prepare a release for version {VERSION}: update version numbers to {VERSION}, ensure CHANGELOG is current, verify tests pass, ensure LICENSE file ({LICENSE:MIT,Apache 2.0,BSD 2-Clause,BSD 3-Clause,ISC,MPL 2.0,Unlicense}) is present, create a release commit, create a git tag v{VERSION}, and run `git push && git push --tags`.".into(),
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
            prompt: "Detect the project type (e.g. Cargo.toml for Rust, package.json for JS/TS, go.mod for Go, etc.), run the appropriate check/lint command, collect all warnings, and fix every one of them.".into(),
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
    #[serde(default)]
    pub claude_env_vars: String,
    #[serde(default)]
    pub claude_pre_run_script: String,
    #[serde(default)]
    pub claude_post_run_script: String,
    pub opencode_model: String,
    #[serde(default)]
    pub opencode_cli_path: String,
    #[serde(default)]
    pub opencode_extra_args: String,
    #[serde(default)]
    pub opencode_env_vars: String,
    #[serde(default)]
    pub opencode_pre_run_script: String,
    #[serde(default)]
    pub opencode_post_run_script: String,
    pub recent_repos: Vec<String>,
    #[serde(default = "default_true")]
    pub notify_sound: bool,
    #[serde(default = "default_true")]
    pub notify_popup: bool,
    #[serde(default = "default_true")]
    pub lava_lamp_enabled: bool,
    #[serde(default = "default_font_family")]
    pub font_family: String,
    #[serde(default = "default_font_size")]
    pub font_size: f32,
    #[serde(default)]
    pub sources: Vec<SourceConfig>,
    #[serde(default = "default_playbook")]
    pub playbook: Vec<Play>,
    /// Allow running Claude/OpenCode when the project root is the user's home folder.
    /// Disabled by default to prevent the AI from reading personal folders like
    /// Documents, Desktop, Photos, etc.
    #[serde(default)]
    pub allow_home_folder_access: bool,
    /// Shell init snippet prepended to every agent command (e.g. `source ~/.zprofile`).
    /// Solves the macOS GUI-app problem where PATH / JAVA_HOME etc. are not set.
    #[serde(default)]
    pub agent_shell_init: String,
    #[serde(default = "default_agents")]
    pub agents: Vec<AgentConfig>,
    /// Command modes triggered by `[name]` prefix in cue text.
    #[serde(default = "default_commands")]
    pub commands: Vec<CueCommand>,
    /// Show heuristic prompt-refinement suggestions below the prompt field.
    #[serde(default)]
    pub prompt_suggestions_enabled: bool,
    /// Automatically include file content (±50 lines) around the cue location in the prompt.
    #[serde(default)]
    pub auto_context_file: bool,
    /// Automatically include the git diff in the prompt.
    #[serde(default)]
    pub auto_context_git_diff: bool,
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
            claude_env_vars: String::new(),
            claude_pre_run_script: String::new(),
            claude_post_run_script: String::new(),
            opencode_model: "openai/o1".to_string(),
            opencode_cli_path: String::new(),
            opencode_extra_args: String::new(),
            opencode_env_vars: String::new(),
            opencode_pre_run_script: String::new(),
            opencode_post_run_script: String::new(),
            recent_repos: Vec::new(),
            notify_sound: true,
            notify_popup: true,
            lava_lamp_enabled: true,
            font_family: default_font_family(),
            font_size: default_font_size(),
            sources: Vec::new(),
            playbook: default_playbook(),
            allow_home_folder_access: false,
            agent_shell_init: String::new(),
            agents: default_agents(),
            commands: default_commands(),
            prompt_suggestions_enabled: false,
            auto_context_file: false,
            auto_context_git_diff: false,
        }
    }
}

/// Resolve the full path for a CLI tool.
///
/// macOS `.app` bundles inherit a minimal PATH (`/usr/bin:/bin:…`), so a plain
/// `which` won't find tools installed via Homebrew, npm, etc.  We therefore:
///   1. Try a login-shell `which` (`zsh -l -c 'which <name>'`) to pick up the
///      user's full PATH from their shell profile.
///   2. Fall back to a plain `which` (works when launched from a terminal).
///   3. Probe well-known installation directories as a last resort.
fn which(name: &str) -> Option<String> {
    // 1. Login shell — picks up ~/.zprofile, ~/.zshrc, etc.
    let login = std::process::Command::new("/bin/zsh")
        .args(["-l", "-c", &format!("which {name}")])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty());
    if login.is_some() {
        return login;
    }

    // 2. Plain which (limited PATH, but works from terminal launches).
    let plain = std::process::Command::new("which")
        .arg(name)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty());
    if plain.is_some() {
        return plain;
    }

    // 3. Well-known paths (Homebrew, npm global, user-local).
    let home = std::env::var("HOME").unwrap_or_default();
    let candidates = [
        format!("/opt/homebrew/bin/{name}"),
        format!("/usr/local/bin/{name}"),
        format!("{home}/.local/bin/{name}"),
        format!("{home}/.npm-global/bin/{name}"),
        format!("{home}/.nvm/current/bin/{name}"),
    ];
    for p in &candidates {
        if std::path::Path::new(p).is_file() {
            return Some(p.clone());
        }
    }

    None
}

pub(crate) fn load_settings(project_root: &Path) -> Settings {
    let path = project_root.join(".Dirigent").join("settings.json");
    let mut settings = match std::fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => Settings::default(),
    };
    // Auto-detect CLI paths on first launch (when paths are empty)
    if settings.claude_cli_path.is_empty() {
        if let Some(path) = which("claude") {
            settings.claude_cli_path = path;
        }
    }
    if settings.opencode_cli_path.is_empty() {
        if let Some(path) = which("opencode") {
            settings.opencode_cli_path = path;
        }
    }
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
    // Append any new default commands that aren't already defined
    for default_cmd in default_commands() {
        if !settings.commands.iter().any(|c| c.name == default_cmd.name) {
            settings.commands.push(default_cmd);
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

/// Install or remove the Claude Code home-directory guard hook.
///
/// When `allow_home_folder_access` is **false**, writes a `PreToolUse` hook
/// script to `.Dirigent/home_guard.sh` and registers it in
/// `.claude/settings.local.json` so Claude Code blocks tool calls that try to
/// read personal directories like ~/Documents, ~/Desktop, ~/Photos, etc.
///
/// When `allow_home_folder_access` is **true**, removes the hook script and
/// its registration from the settings file.
pub(crate) fn sync_home_guard_hook(project_root: &Path, allow: bool) {
    let dirigent_dir = project_root.join(".Dirigent");
    let claude_dir = project_root.join(".claude");
    let hook_script = dirigent_dir.join("home_guard.sh");
    let settings_file = claude_dir.join("settings.local.json");

    if allow {
        // --- Remove the hook ---
        let _ = std::fs::remove_file(&hook_script);
        remove_hook_from_settings(&settings_file);
    } else {
        // --- Install the hook ---
        let _ = std::fs::create_dir_all(&dirigent_dir);
        let _ = std::fs::write(&hook_script, home_guard_script_content());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&hook_script, std::fs::Permissions::from_mode(0o755));
        }
        let _ = std::fs::create_dir_all(&claude_dir);
        upsert_hook_in_settings(&settings_file, &hook_script);
    }
}

/// The shell script that Claude Code runs as a `PreToolUse` hook.
/// It checks every path-like value in the tool input JSON against a set of
/// restricted home sub-directories and exits with code 2 to block the call.
fn home_guard_script_content() -> String {
    r#"#!/bin/bash
# Dirigent home-directory guard – Claude Code PreToolUse hook.
# Blocks tool calls that reference personal home directories or
# recursively search from the home directory root.
INPUT=$(cat)
HOME_DIR="${HOME:-/Users/$(whoami)}"

# 1. Block explicit references to personal sub-directories.
for DIR in Documents Desktop Downloads Photos Pictures Movies Music Library Applications .ssh .gnupg; do
    BLOCKED="$HOME_DIR/$DIR"
    if echo "$INPUT" | grep -qF "$BLOCKED"; then
        echo "Blocked by Dirigent: access to ~/$DIR is restricted. Disable the home-folder guard in Dirigent Settings to override."
        exit 2
    fi
done

# 2. Block recursive commands that start from the home directory itself
#    (e.g. "find /Users/lars -name foo" or "find ~ -type f").
#    These traverse into Documents, Desktop, Photos etc. and trigger macOS
#    permission pop-ups even though those paths aren't named explicitly.
#    We match: find <home> | find ~ | ls -R <home> | grep -r ... <home>
#    but NOT paths that go deeper (e.g. find /Users/lars/prj is fine).
HOME_ESC=$(printf '%s' "$HOME_DIR" | sed 's/[.[\*^$()+?{|]/\\&/g')
if echo "$INPUT" | grep -qE "(find|ls -[a-zA-Z]*R|grep -[a-zA-Z]*r|rg |fd |du |tree )[^\"]*($HOME_ESC|~)(/| |\"|\$)" 2>/dev/null; then
    # Make sure it's not targeting a deeper subdirectory within home
    if ! echo "$INPUT" | grep -qE "(find|ls|grep|rg|fd|du|tree)[^\"]*$HOME_ESC/[A-Za-z0-9._-]+[/ \"]" 2>/dev/null; then
        echo "Blocked by Dirigent: recursive search from home directory is restricted. Use a more specific path or disable the home-folder guard in Dirigent Settings."
        exit 2
    fi
fi

exit 0
"#
    .to_string()
}

/// Add (or re-add) our guard hook entry to a `.claude/settings.local.json` file.
fn upsert_hook_in_settings(settings_path: &Path, hook_script: &Path) {
    let mut root = read_json_object(settings_path);

    let hooks = root
        .as_object_mut()
        .unwrap()
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}));

    if !hooks.is_object() {
        *hooks = serde_json::json!({});
    }

    let pre = hooks
        .as_object_mut()
        .unwrap()
        .entry("PreToolUse")
        .or_insert_with(|| serde_json::json!([]));

    if let Some(arr) = pre.as_array_mut() {
        // Remove any previous guard entry
        arr.retain(|h| !h.to_string().contains("home_guard.sh"));
        // Add the new entry
        arr.push(serde_json::json!({
            "matcher": "",
            "hooks": [{
                "type": "command",
                "command": hook_script.to_string_lossy().to_string()
            }]
        }));
    }

    if let Ok(json) = serde_json::to_string_pretty(&root) {
        let _ = std::fs::write(settings_path, json);
    }
}

/// Remove our guard hook entry from a `.claude/settings.local.json` file.
fn remove_hook_from_settings(settings_path: &Path) {
    if !settings_path.exists() {
        return;
    }
    let mut root = read_json_object(settings_path);

    let changed = if let Some(hooks) = root.get_mut("hooks") {
        if let Some(pre) = hooks.get_mut("PreToolUse") {
            if let Some(arr) = pre.as_array_mut() {
                let before = arr.len();
                arr.retain(|h| !h.to_string().contains("home_guard.sh"));
                arr.len() != before
            } else {
                false
            }
        } else {
            false
        }
    } else {
        false
    };

    if changed {
        if let Ok(json) = serde_json::to_string_pretty(&root) {
            let _ = std::fs::write(settings_path, json);
        }
    }
}

/// Read a JSON file as a `serde_json::Value` object, defaulting to `{}`.
fn read_json_object(path: &Path) -> serde_json::Value {
    if path.exists() {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|c| serde_json::from_str(&c).ok())
            .unwrap_or_else(|| serde_json::json!({}))
    } else {
        serde_json::json!({})
    }
}

pub(crate) fn add_recent_repo(settings: &mut Settings, path: &str) {
    settings.recent_repos.retain(|p| p != path);
    settings.recent_repos.insert(0, path.to_string());
    settings.recent_repos.truncate(10);
}

// ---------------------------------------------------------------------------
// Global recent-projects list (persisted across all projects / app launches)
// ---------------------------------------------------------------------------

/// Returns the path to the global recent-projects file:
/// `~/Library/Application Support/Dirigent/recent_projects.json`
fn global_recent_projects_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join("Library/Application Support/Dirigent/recent_projects.json"))
}

/// Load the global list of recently opened project paths.
pub(crate) fn load_global_recent_projects() -> Vec<String> {
    let path = match global_recent_projects_path() {
        Some(p) => p,
        None => return Vec::new(),
    };
    match std::fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// Persist the global list of recently opened project paths.
pub(crate) fn save_global_recent_projects(projects: &[String]) {
    let path = match global_recent_projects_path() {
        Some(p) => p,
        None => return,
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(projects) {
        let _ = std::fs::write(path, json);
    }
}

/// Add a project path to the global recent list and persist it.
pub(crate) fn add_global_recent_project(path: &str) {
    let mut projects = load_global_recent_projects();
    projects.retain(|p| p != path);
    projects.insert(0, path.to_string());
    projects.truncate(20);
    save_global_recent_projects(&projects);
}
