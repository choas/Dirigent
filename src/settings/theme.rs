use eframe::egui;
use serde::{Deserialize, Serialize};

use super::semantic_colors::SemanticColors;

/// A user-defined custom theme with explicit RGB palette colors.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct CustomTheme {
    pub name: String,
    pub is_dark: bool,
    pub panel_fill: [u8; 3],
    pub window_fill: [u8; 3],
    pub extreme_bg: [u8; 3],
    pub faint_bg: [u8; 3],
    pub text: [u8; 3],
    pub selection: [u8; 3],
    pub noninteractive: [u8; 3],
    pub inactive: [u8; 3],
    pub hovered: [u8; 3],
    pub active: [u8; 3],
    pub hyperlink: [u8; 3],
    pub accent: [u8; 3],
}

impl Default for CustomTheme {
    fn default() -> Self {
        Self {
            name: String::new(),
            is_dark: true,
            panel_fill: [32, 33, 38],
            window_fill: [42, 44, 52],
            extreme_bg: [22, 23, 26],
            faint_bg: [38, 40, 46],
            text: [210, 214, 222],
            selection: [42, 62, 110],
            noninteractive: [42, 44, 52],
            inactive: [48, 50, 60],
            hovered: [58, 62, 74],
            active: [100, 180, 255],
            hyperlink: [100, 180, 255],
            accent: [100, 180, 255],
        }
    }
}

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
    // User-defined
    Custom(CustomTheme),
}

impl ThemeChoice {
    pub fn is_dark(&self) -> bool {
        match self {
            ThemeChoice::Custom(ct) => ct.is_dark,
            _ => matches!(
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
            ),
        }
    }

    pub fn display_name(&self) -> &str {
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
            ThemeChoice::Custom(ct) => &ct.name,
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

    /// Convert this theme's palette into a `CustomTheme`, suitable as a starting point.
    pub fn to_custom_theme(&self) -> CustomTheme {
        let p = self.palette();
        let accent = self.semantic_colors().accent;
        let rgb = |c: egui::Color32| [c.r(), c.g(), c.b()];
        CustomTheme {
            name: String::new(),
            is_dark: self.is_dark(),
            panel_fill: rgb(p.panel_fill),
            window_fill: rgb(p.window_fill),
            extreme_bg: rgb(p.extreme_bg),
            faint_bg: rgb(p.faint_bg),
            text: rgb(p.text),
            selection: rgb(p.selection),
            noninteractive: rgb(p.noninteractive),
            inactive: rgb(p.inactive),
            hovered: rgb(p.hovered),
            active: rgb(p.active),
            hyperlink: rgb(p.hyperlink),
            accent: rgb(accent),
        }
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
            Custom(ct) => ThemePalette {
                panel_fill: egui::Color32::from_rgb(
                    ct.panel_fill[0],
                    ct.panel_fill[1],
                    ct.panel_fill[2],
                ),
                window_fill: egui::Color32::from_rgb(
                    ct.window_fill[0],
                    ct.window_fill[1],
                    ct.window_fill[2],
                ),
                extreme_bg: egui::Color32::from_rgb(
                    ct.extreme_bg[0],
                    ct.extreme_bg[1],
                    ct.extreme_bg[2],
                ),
                faint_bg: egui::Color32::from_rgb(ct.faint_bg[0], ct.faint_bg[1], ct.faint_bg[2]),
                text: egui::Color32::from_rgb(ct.text[0], ct.text[1], ct.text[2]),
                selection: egui::Color32::from_rgb(
                    ct.selection[0],
                    ct.selection[1],
                    ct.selection[2],
                ),
                noninteractive: egui::Color32::from_rgb(
                    ct.noninteractive[0],
                    ct.noninteractive[1],
                    ct.noninteractive[2],
                ),
                inactive: egui::Color32::from_rgb(ct.inactive[0], ct.inactive[1], ct.inactive[2]),
                hovered: egui::Color32::from_rgb(ct.hovered[0], ct.hovered[1], ct.hovered[2]),
                active: egui::Color32::from_rgb(ct.active[0], ct.active[1], ct.active[2]),
                hyperlink: egui::Color32::from_rgb(
                    ct.hyperlink[0],
                    ct.hyperlink[1],
                    ct.hyperlink[2],
                ),
            },
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
            Custom(ct) => egui::Color32::from_rgb(ct.accent[0], ct.accent[1], ct.accent[2]),
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
