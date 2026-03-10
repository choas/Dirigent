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
        match self {
            ThemeChoice::Dark => egui::Visuals::dark(),
            ThemeChoice::Light => egui::Visuals::light(),
            _ => self.palette().apply(self.is_dark()),
        }
    }

    fn palette(&self) -> ThemePalette {
        use ThemeChoice::*;
        match self {
            //                            panel_fill         window_fill        extreme_bg         faint_bg           text               selection          noninteractive     inactive           hovered            active             hyperlink
            Nord              => palette!([ 46,  52,  64], [ 59,  66,  82], [ 36,  41,  51], [ 59,  66,  82], [216, 222, 233], [ 94, 129, 172], [ 59,  66,  82], [ 67,  76,  94], [ 76,  86, 106], [ 94, 129, 172], [136, 192, 208]),
            Dracula           => palette!([ 40,  42,  54], [ 68,  71,  90], [ 33,  34,  44], [ 68,  71,  90], [248, 248, 242], [ 68,  71,  90], [ 68,  71,  90], [ 55,  57,  73], [ 80,  83, 105], [189, 147, 249], [139, 233, 253]),
            SolarizedDark     => palette!([  0,  43,  54], [  7,  54,  66], [  0,  34,  43], [  7,  54,  66], [131, 148, 150], [ 38, 139, 210], [  7,  54,  66], [  7,  54,  66], [ 88, 110, 117], [ 38, 139, 210], [ 38, 139, 210]),
            Monokai           => palette!([ 39,  40,  34], [ 49,  50,  44], [ 30,  31,  26], [ 49,  50,  44], [248, 248, 242], [ 73,  72,  62], [ 49,  50,  44], [ 59,  60,  54], [ 73,  72,  62], [166, 226,  46], [102, 217, 239]),
            GruvboxDark       => palette!([ 40,  40,  40], [ 50,  48,  47], [ 29,  32,  33], [ 50,  48,  47], [235, 219, 178], [ 69, 133, 136], [ 50,  48,  47], [ 60,  56,  54], [ 80,  73,  69], [152, 151,  26], [131, 165, 152]),
            TokyoNight        => palette!([ 26,  27,  38], [ 36,  40,  59], [ 22,  22,  30], [ 36,  40,  59], [169, 177, 214], [ 42,  47,  78], [ 36,  40,  59], [ 41,  46,  73], [ 52,  59,  88], [122, 162, 247], [125, 207, 255]),
            OneDark           => palette!([ 40,  44,  52], [ 50,  55,  65], [ 33,  37,  43], [ 50,  55,  65], [171, 178, 191], [ 62,  68,  81], [ 50,  55,  65], [ 55,  60,  72], [ 62,  68,  81], [ 97, 175, 239], [ 86, 182, 194]),
            CatppuccinMocha   => palette!([ 30,  30,  46], [ 49,  50,  68], [ 17,  17,  27], [ 24,  24,  37], [205, 214, 244], [ 88,  91, 112], [ 49,  50,  68], [ 69,  71,  90], [ 88,  91, 112], [137, 180, 250], [116, 199, 236]),
            EverforestDark    => palette!([ 47,  53,  57], [ 52,  58,  62], [ 39,  44,  48], [ 52,  58,  62], [211, 198, 170], [ 80,  88,  92], [ 52,  58,  62], [ 58,  65,  68], [ 68,  75,  79], [167, 192, 128], [127, 187, 179]),
            SolarizedLight    => palette!([253, 246, 227], [238, 232, 213], [255, 250, 235], [238, 232, 213], [101, 123, 131], [ 38, 139, 210], [238, 232, 213], [238, 232, 213], [220, 215, 198], [ 38, 139, 210], [ 38, 139, 210]),
            GruvboxLight      => palette!([251, 241, 199], [242, 229, 188], [255, 248, 210], [242, 229, 188], [ 60,  56,  54], [ 69, 133, 136], [242, 229, 188], [235, 219, 178], [213, 196, 161], [152, 151,  26], [  7, 102, 120]),
            GitHubLight       => palette!([255, 255, 255], [246, 248, 250], [255, 255, 255], [246, 248, 250], [ 36,  41,  46], [  0,  92, 197], [246, 248, 250], [234, 238, 242], [220, 224, 228], [  0,  92, 197], [  3,  47,  98]),
            CatppuccinLatte   => palette!([239, 241, 245], [204, 208, 218], [220, 224, 232], [230, 233, 239], [ 76,  79, 105], [172, 176, 190], [204, 208, 218], [188, 192, 204], [172, 176, 190], [ 30, 102, 245], [ 32, 159, 181]),
            EverforestLight   => palette!([253, 246, 227], [242, 237, 220], [255, 252, 238], [242, 237, 220], [ 92,  99,  78], [160, 188, 132], [242, 237, 220], [230, 225, 208], [218, 213, 196], [141, 165, 104], [ 53, 162, 147]),
            RosePineDawn      => palette!([250, 244, 237], [255, 250, 243], [242, 233, 222], [255, 250, 243], [ 87,  82, 121], [215, 130, 126], [255, 250, 243], [242, 233, 222], [232, 222, 210], [ 87,  82, 121], [ 40, 105, 131]),
            OneLight          => palette!([250, 250, 250], [240, 240, 240], [255, 255, 255], [240, 240, 240], [ 56,  58,  66], [198, 216, 240], [240, 240, 240], [232, 232, 232], [218, 218, 218], [ 64, 120, 242], [  1, 132, 188]),
            NordLight         => palette!([236, 239, 244], [229, 233, 240], [242, 245, 250], [229, 233, 240], [ 59,  66,  82], [136, 192, 208], [229, 233, 240], [216, 222, 233], [208, 214, 225], [ 94, 129, 172], [ 94, 129, 172]),
            TokyoNightLight   => palette!([213, 214, 219], [224, 225, 228], [235, 236, 240], [224, 225, 228], [ 52,  54,  86], [180, 182, 200], [224, 225, 228], [210, 211, 216], [198, 199, 206], [ 52,  84, 223], [118, 105, 199]),
            Dark | Light => unreachable!(),
        }
    }
}

macro_rules! palette {
    ([$pr:expr,$pg:expr,$pb:expr], [$wr:expr,$wg:expr,$wb:expr], [$er:expr,$eg:expr,$eb:expr], [$fr:expr,$fg:expr,$fb:expr], [$tr:expr,$tg:expr,$tb:expr], [$sr:expr,$sg:expr,$sb:expr], [$nr:expr,$ng:expr,$nb:expr], [$ir:expr,$ig:expr,$ib:expr], [$hr:expr,$hg:expr,$hb:expr], [$ar:expr,$ag:expr,$ab:expr], [$lr:expr,$lg:expr,$lb:expr]) => {
        ThemePalette {
            panel_fill:    egui::Color32::from_rgb($pr, $pg, $pb),
            window_fill:   egui::Color32::from_rgb($wr, $wg, $wb),
            extreme_bg:    egui::Color32::from_rgb($er, $eg, $eb),
            faint_bg:      egui::Color32::from_rgb($fr, $fg, $fb),
            text:          egui::Color32::from_rgb($tr, $tg, $tb),
            selection:     egui::Color32::from_rgb($sr, $sg, $sb),
            noninteractive:egui::Color32::from_rgb($nr, $ng, $nb),
            inactive:      egui::Color32::from_rgb($ir, $ig, $ib),
            hovered:       egui::Color32::from_rgb($hr, $hg, $hb),
            active:        egui::Color32::from_rgb($ar, $ag, $ab),
            hyperlink:     egui::Color32::from_rgb($lr, $lg, $lb),
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

impl ThemePalette {
    fn apply(self, dark: bool) -> egui::Visuals {
        let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
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
        v
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
            prompt: "Commit all current changes. First, check the SQLite database for cues that are in 'review' or 'done' status. Use their titles/descriptions to write a meaningful commit message summarizing what was done. Move any 'review' cues to 'done' status. Then stage all changes and create the commit.".into(),
        },
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Settings {
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
            claude_model: "claude-opus-4-6".to_string(),
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
    match std::fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => Settings::default(),
    }
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
