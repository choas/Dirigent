use eframe::egui;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ThemeChoice {
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
        match self {
            ThemeChoice::Dark => egui::Visuals::dark(),
            ThemeChoice::Light => egui::Visuals::light(),
            ThemeChoice::Nord => {
                let mut v = egui::Visuals::dark();
                v.panel_fill = egui::Color32::from_rgb(46, 52, 64);       // Nord0
                v.window_fill = egui::Color32::from_rgb(59, 66, 82);      // Nord1
                v.extreme_bg_color = egui::Color32::from_rgb(36, 41, 51);
                v.faint_bg_color = egui::Color32::from_rgb(59, 66, 82);
                v.override_text_color = Some(egui::Color32::from_rgb(216, 222, 233)); // Nord4
                v.selection.bg_fill = egui::Color32::from_rgb(94, 129, 172);           // Nord10
                v.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(59, 66, 82);
                v.widgets.inactive.bg_fill = egui::Color32::from_rgb(67, 76, 94);     // Nord2
                v.widgets.hovered.bg_fill = egui::Color32::from_rgb(76, 86, 106);     // Nord3
                v.widgets.active.bg_fill = egui::Color32::from_rgb(94, 129, 172);
                v.hyperlink_color = egui::Color32::from_rgb(136, 192, 208);            // Nord8
                v
            }
            ThemeChoice::Dracula => {
                let mut v = egui::Visuals::dark();
                v.panel_fill = egui::Color32::from_rgb(40, 42, 54);       // Background
                v.window_fill = egui::Color32::from_rgb(68, 71, 90);      // Current Line
                v.extreme_bg_color = egui::Color32::from_rgb(33, 34, 44);
                v.faint_bg_color = egui::Color32::from_rgb(68, 71, 90);
                v.override_text_color = Some(egui::Color32::from_rgb(248, 248, 242)); // Foreground
                v.selection.bg_fill = egui::Color32::from_rgb(68, 71, 90);
                v.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(68, 71, 90);
                v.widgets.inactive.bg_fill = egui::Color32::from_rgb(55, 57, 73);
                v.widgets.hovered.bg_fill = egui::Color32::from_rgb(80, 83, 105);
                v.widgets.active.bg_fill = egui::Color32::from_rgb(189, 147, 249);    // Purple
                v.hyperlink_color = egui::Color32::from_rgb(139, 233, 253);            // Cyan
                v
            }
            ThemeChoice::SolarizedDark => {
                let mut v = egui::Visuals::dark();
                v.panel_fill = egui::Color32::from_rgb(0, 43, 54);        // base03
                v.window_fill = egui::Color32::from_rgb(7, 54, 66);       // base02
                v.extreme_bg_color = egui::Color32::from_rgb(0, 34, 43);
                v.faint_bg_color = egui::Color32::from_rgb(7, 54, 66);
                v.override_text_color = Some(egui::Color32::from_rgb(131, 148, 150)); // base0
                v.selection.bg_fill = egui::Color32::from_rgb(38, 139, 210);           // blue
                v.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(7, 54, 66);
                v.widgets.inactive.bg_fill = egui::Color32::from_rgb(7, 54, 66);
                v.widgets.hovered.bg_fill = egui::Color32::from_rgb(88, 110, 117);    // base01
                v.widgets.active.bg_fill = egui::Color32::from_rgb(38, 139, 210);
                v.hyperlink_color = egui::Color32::from_rgb(38, 139, 210);
                v
            }
            ThemeChoice::Monokai => {
                let mut v = egui::Visuals::dark();
                v.panel_fill = egui::Color32::from_rgb(39, 40, 34);
                v.window_fill = egui::Color32::from_rgb(49, 50, 44);
                v.extreme_bg_color = egui::Color32::from_rgb(30, 31, 26);
                v.faint_bg_color = egui::Color32::from_rgb(49, 50, 44);
                v.override_text_color = Some(egui::Color32::from_rgb(248, 248, 242));
                v.selection.bg_fill = egui::Color32::from_rgb(73, 72, 62);
                v.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(49, 50, 44);
                v.widgets.inactive.bg_fill = egui::Color32::from_rgb(59, 60, 54);
                v.widgets.hovered.bg_fill = egui::Color32::from_rgb(73, 72, 62);
                v.widgets.active.bg_fill = egui::Color32::from_rgb(166, 226, 46);     // Green
                v.hyperlink_color = egui::Color32::from_rgb(102, 217, 239);            // Cyan
                v
            }
            ThemeChoice::GruvboxDark => {
                let mut v = egui::Visuals::dark();
                v.panel_fill = egui::Color32::from_rgb(40, 40, 40);       // bg
                v.window_fill = egui::Color32::from_rgb(50, 48, 47);      // bg1
                v.extreme_bg_color = egui::Color32::from_rgb(29, 32, 33); // bg0_h
                v.faint_bg_color = egui::Color32::from_rgb(50, 48, 47);
                v.override_text_color = Some(egui::Color32::from_rgb(235, 219, 178)); // fg
                v.selection.bg_fill = egui::Color32::from_rgb(69, 133, 136);           // aqua
                v.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(50, 48, 47);
                v.widgets.inactive.bg_fill = egui::Color32::from_rgb(60, 56, 54);     // bg2
                v.widgets.hovered.bg_fill = egui::Color32::from_rgb(80, 73, 69);      // bg3
                v.widgets.active.bg_fill = egui::Color32::from_rgb(152, 151, 26);     // green
                v.hyperlink_color = egui::Color32::from_rgb(131, 165, 152);            // aqua2
                v
            }
            ThemeChoice::TokyoNight => {
                let mut v = egui::Visuals::dark();
                v.panel_fill = egui::Color32::from_rgb(26, 27, 38);
                v.window_fill = egui::Color32::from_rgb(36, 40, 59);
                v.extreme_bg_color = egui::Color32::from_rgb(22, 22, 30);
                v.faint_bg_color = egui::Color32::from_rgb(36, 40, 59);
                v.override_text_color = Some(egui::Color32::from_rgb(169, 177, 214));
                v.selection.bg_fill = egui::Color32::from_rgb(42, 47, 78);
                v.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(36, 40, 59);
                v.widgets.inactive.bg_fill = egui::Color32::from_rgb(41, 46, 73);
                v.widgets.hovered.bg_fill = egui::Color32::from_rgb(52, 59, 88);
                v.widgets.active.bg_fill = egui::Color32::from_rgb(122, 162, 247);    // Blue
                v.hyperlink_color = egui::Color32::from_rgb(125, 207, 255);
                v
            }
            ThemeChoice::OneDark => {
                let mut v = egui::Visuals::dark();
                v.panel_fill = egui::Color32::from_rgb(40, 44, 52);
                v.window_fill = egui::Color32::from_rgb(50, 55, 65);
                v.extreme_bg_color = egui::Color32::from_rgb(33, 37, 43);
                v.faint_bg_color = egui::Color32::from_rgb(50, 55, 65);
                v.override_text_color = Some(egui::Color32::from_rgb(171, 178, 191));
                v.selection.bg_fill = egui::Color32::from_rgb(62, 68, 81);
                v.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(50, 55, 65);
                v.widgets.inactive.bg_fill = egui::Color32::from_rgb(55, 60, 72);
                v.widgets.hovered.bg_fill = egui::Color32::from_rgb(62, 68, 81);
                v.widgets.active.bg_fill = egui::Color32::from_rgb(97, 175, 239);     // Blue
                v.hyperlink_color = egui::Color32::from_rgb(86, 182, 194);             // Cyan
                v
            }
            ThemeChoice::CatppuccinMocha => {
                let mut v = egui::Visuals::dark();
                v.panel_fill = egui::Color32::from_rgb(30, 30, 46);       // Base
                v.window_fill = egui::Color32::from_rgb(49, 50, 68);      // Surface0
                v.extreme_bg_color = egui::Color32::from_rgb(17, 17, 27); // Crust
                v.faint_bg_color = egui::Color32::from_rgb(24, 24, 37);   // Mantle
                v.override_text_color = Some(egui::Color32::from_rgb(205, 214, 244)); // Text
                v.selection.bg_fill = egui::Color32::from_rgb(88, 91, 112);            // Surface2
                v.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(49, 50, 68);
                v.widgets.inactive.bg_fill = egui::Color32::from_rgb(69, 71, 90);     // Surface1
                v.widgets.hovered.bg_fill = egui::Color32::from_rgb(88, 91, 112);
                v.widgets.active.bg_fill = egui::Color32::from_rgb(137, 180, 250);    // Blue
                v.hyperlink_color = egui::Color32::from_rgb(116, 199, 236);            // Sapphire
                v
            }
            ThemeChoice::EverforestDark => {
                let mut v = egui::Visuals::dark();
                v.panel_fill = egui::Color32::from_rgb(47, 53, 57);       // bg0
                v.window_fill = egui::Color32::from_rgb(52, 58, 62);      // bg1
                v.extreme_bg_color = egui::Color32::from_rgb(39, 44, 48);
                v.faint_bg_color = egui::Color32::from_rgb(52, 58, 62);
                v.override_text_color = Some(egui::Color32::from_rgb(211, 198, 170)); // fg
                v.selection.bg_fill = egui::Color32::from_rgb(80, 88, 92);
                v.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(52, 58, 62);
                v.widgets.inactive.bg_fill = egui::Color32::from_rgb(58, 65, 68);     // bg2
                v.widgets.hovered.bg_fill = egui::Color32::from_rgb(68, 75, 79);      // bg3
                v.widgets.active.bg_fill = egui::Color32::from_rgb(167, 192, 128);    // green
                v.hyperlink_color = egui::Color32::from_rgb(127, 187, 179);            // aqua
                v
            }
            // Light themes
            ThemeChoice::SolarizedLight => {
                let mut v = egui::Visuals::light();
                v.panel_fill = egui::Color32::from_rgb(253, 246, 227);     // base3
                v.window_fill = egui::Color32::from_rgb(238, 232, 213);    // base2
                v.extreme_bg_color = egui::Color32::from_rgb(255, 250, 235);
                v.faint_bg_color = egui::Color32::from_rgb(238, 232, 213);
                v.override_text_color = Some(egui::Color32::from_rgb(101, 123, 131)); // base00
                v.selection.bg_fill = egui::Color32::from_rgb(38, 139, 210);
                v.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(238, 232, 213);
                v.widgets.inactive.bg_fill = egui::Color32::from_rgb(238, 232, 213);
                v.widgets.hovered.bg_fill = egui::Color32::from_rgb(220, 215, 198);
                v.widgets.active.bg_fill = egui::Color32::from_rgb(38, 139, 210);
                v.hyperlink_color = egui::Color32::from_rgb(38, 139, 210);
                v
            }
            ThemeChoice::GruvboxLight => {
                let mut v = egui::Visuals::light();
                v.panel_fill = egui::Color32::from_rgb(251, 241, 199);     // bg
                v.window_fill = egui::Color32::from_rgb(242, 229, 188);    // bg1
                v.extreme_bg_color = egui::Color32::from_rgb(255, 248, 210);
                v.faint_bg_color = egui::Color32::from_rgb(242, 229, 188);
                v.override_text_color = Some(egui::Color32::from_rgb(60, 56, 54));     // fg
                v.selection.bg_fill = egui::Color32::from_rgb(69, 133, 136);
                v.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(242, 229, 188);
                v.widgets.inactive.bg_fill = egui::Color32::from_rgb(235, 219, 178);  // bg2
                v.widgets.hovered.bg_fill = egui::Color32::from_rgb(213, 196, 161);   // bg3
                v.widgets.active.bg_fill = egui::Color32::from_rgb(152, 151, 26);
                v.hyperlink_color = egui::Color32::from_rgb(7, 102, 120);              // aqua
                v
            }
            ThemeChoice::GitHubLight => {
                let mut v = egui::Visuals::light();
                v.panel_fill = egui::Color32::from_rgb(255, 255, 255);
                v.window_fill = egui::Color32::from_rgb(246, 248, 250);
                v.extreme_bg_color = egui::Color32::from_rgb(255, 255, 255);
                v.faint_bg_color = egui::Color32::from_rgb(246, 248, 250);
                v.override_text_color = Some(egui::Color32::from_rgb(36, 41, 46));
                v.selection.bg_fill = egui::Color32::from_rgb(0, 92, 197);
                v.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(246, 248, 250);
                v.widgets.inactive.bg_fill = egui::Color32::from_rgb(234, 238, 242);
                v.widgets.hovered.bg_fill = egui::Color32::from_rgb(220, 224, 228);
                v.widgets.active.bg_fill = egui::Color32::from_rgb(0, 92, 197);
                v.hyperlink_color = egui::Color32::from_rgb(3, 47, 98);
                v
            }
            ThemeChoice::CatppuccinLatte => {
                let mut v = egui::Visuals::light();
                v.panel_fill = egui::Color32::from_rgb(239, 241, 245);     // Base
                v.window_fill = egui::Color32::from_rgb(204, 208, 218);    // Surface0
                v.extreme_bg_color = egui::Color32::from_rgb(220, 224, 232); // Crust
                v.faint_bg_color = egui::Color32::from_rgb(230, 233, 239); // Mantle
                v.override_text_color = Some(egui::Color32::from_rgb(76, 79, 105));   // Text
                v.selection.bg_fill = egui::Color32::from_rgb(172, 176, 190);
                v.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(204, 208, 218);
                v.widgets.inactive.bg_fill = egui::Color32::from_rgb(188, 192, 204);  // Surface1
                v.widgets.hovered.bg_fill = egui::Color32::from_rgb(172, 176, 190);   // Surface2
                v.widgets.active.bg_fill = egui::Color32::from_rgb(30, 102, 245);     // Blue
                v.hyperlink_color = egui::Color32::from_rgb(32, 159, 181);             // Sapphire
                v
            }
            ThemeChoice::EverforestLight => {
                let mut v = egui::Visuals::light();
                v.panel_fill = egui::Color32::from_rgb(253, 246, 227);     // bg0
                v.window_fill = egui::Color32::from_rgb(242, 237, 220);    // bg1
                v.extreme_bg_color = egui::Color32::from_rgb(255, 252, 238);
                v.faint_bg_color = egui::Color32::from_rgb(242, 237, 220);
                v.override_text_color = Some(egui::Color32::from_rgb(92, 99, 78));     // fg
                v.selection.bg_fill = egui::Color32::from_rgb(160, 188, 132);
                v.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(242, 237, 220);
                v.widgets.inactive.bg_fill = egui::Color32::from_rgb(230, 225, 208);  // bg2
                v.widgets.hovered.bg_fill = egui::Color32::from_rgb(218, 213, 196);   // bg3
                v.widgets.active.bg_fill = egui::Color32::from_rgb(141, 165, 104);    // green
                v.hyperlink_color = egui::Color32::from_rgb(53, 162, 147);             // aqua
                v
            }
            ThemeChoice::RosePineDawn => {
                let mut v = egui::Visuals::light();
                v.panel_fill = egui::Color32::from_rgb(250, 244, 237);     // base
                v.window_fill = egui::Color32::from_rgb(255, 250, 243);    // surface
                v.extreme_bg_color = egui::Color32::from_rgb(242, 233, 222); // overlay
                v.faint_bg_color = egui::Color32::from_rgb(255, 250, 243);
                v.override_text_color = Some(egui::Color32::from_rgb(87, 82, 121));   // text
                v.selection.bg_fill = egui::Color32::from_rgb(215, 130, 126);          // rose
                v.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(255, 250, 243);
                v.widgets.inactive.bg_fill = egui::Color32::from_rgb(242, 233, 222);
                v.widgets.hovered.bg_fill = egui::Color32::from_rgb(232, 222, 210);
                v.widgets.active.bg_fill = egui::Color32::from_rgb(87, 82, 121);      // iris
                v.hyperlink_color = egui::Color32::from_rgb(40, 105, 131);             // pine
                v
            }
            ThemeChoice::OneLight => {
                let mut v = egui::Visuals::light();
                v.panel_fill = egui::Color32::from_rgb(250, 250, 250);
                v.window_fill = egui::Color32::from_rgb(240, 240, 240);
                v.extreme_bg_color = egui::Color32::from_rgb(255, 255, 255);
                v.faint_bg_color = egui::Color32::from_rgb(240, 240, 240);
                v.override_text_color = Some(egui::Color32::from_rgb(56, 58, 66));
                v.selection.bg_fill = egui::Color32::from_rgb(198, 216, 240);
                v.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(240, 240, 240);
                v.widgets.inactive.bg_fill = egui::Color32::from_rgb(232, 232, 232);
                v.widgets.hovered.bg_fill = egui::Color32::from_rgb(218, 218, 218);
                v.widgets.active.bg_fill = egui::Color32::from_rgb(64, 120, 242);
                v.hyperlink_color = egui::Color32::from_rgb(1, 132, 188);
                v
            }
            ThemeChoice::NordLight => {
                let mut v = egui::Visuals::light();
                v.panel_fill = egui::Color32::from_rgb(236, 239, 244);     // Nord6
                v.window_fill = egui::Color32::from_rgb(229, 233, 240);    // Nord5
                v.extreme_bg_color = egui::Color32::from_rgb(242, 245, 250);
                v.faint_bg_color = egui::Color32::from_rgb(229, 233, 240);
                v.override_text_color = Some(egui::Color32::from_rgb(59, 66, 82));     // Nord1
                v.selection.bg_fill = egui::Color32::from_rgb(136, 192, 208);           // Nord7
                v.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(229, 233, 240);
                v.widgets.inactive.bg_fill = egui::Color32::from_rgb(216, 222, 233);  // Nord4
                v.widgets.hovered.bg_fill = egui::Color32::from_rgb(208, 214, 225);
                v.widgets.active.bg_fill = egui::Color32::from_rgb(94, 129, 172);     // Nord10
                v.hyperlink_color = egui::Color32::from_rgb(94, 129, 172);
                v
            }
            ThemeChoice::TokyoNightLight => {
                let mut v = egui::Visuals::light();
                v.panel_fill = egui::Color32::from_rgb(213, 214, 219);     // bg
                v.window_fill = egui::Color32::from_rgb(224, 225, 228);    // bg_highlight
                v.extreme_bg_color = egui::Color32::from_rgb(235, 236, 240);
                v.faint_bg_color = egui::Color32::from_rgb(224, 225, 228);
                v.override_text_color = Some(egui::Color32::from_rgb(52, 54, 86));     // fg
                v.selection.bg_fill = egui::Color32::from_rgb(180, 182, 200);
                v.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(224, 225, 228);
                v.widgets.inactive.bg_fill = egui::Color32::from_rgb(210, 211, 216);
                v.widgets.hovered.bg_fill = egui::Color32::from_rgb(198, 199, 206);
                v.widgets.active.bg_fill = egui::Color32::from_rgb(52, 84, 223);
                v.hyperlink_color = egui::Color32::from_rgb(118, 105, 199);
                v
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SourceKind {
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
pub struct SourceConfig {
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
pub struct Settings {
    pub theme: ThemeChoice,
    pub claude_model: String,
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
            claude_model: "claude-opus-4-6".to_string(),
            recent_repos: Vec::new(),
            notify_sound: true,
            notify_popup: true,
            font_family: default_font_family(),
            font_size: default_font_size(),
            sources: Vec::new(),
        }
    }
}

pub fn load_settings(project_root: &Path) -> Settings {
    let path = project_root.join(".Dirigent").join("settings.json");
    match std::fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => Settings::default(),
    }
}

pub fn save_settings(project_root: &Path, settings: &Settings) {
    let dir = project_root.join(".Dirigent");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("settings.json");
    if let Ok(json) = serde_json::to_string_pretty(settings) {
        let _ = std::fs::write(path, json);
    }
}

pub fn add_recent_repo(settings: &mut Settings, path: &str) {
    settings.recent_repos.retain(|p| p != path);
    settings.recent_repos.insert(0, path.to_string());
    settings.recent_repos.truncate(10);
}
