use std::fmt;

use eframe::egui;

use crate::app::{AsyncPoll, CustomThemeEdit, DirigentApp, SPACE_MD, SPACE_SM, SPACE_XS};
use crate::settings::{CliProvider, CustomTheme};

#[derive(Default)]
struct CustomThemeDialogActions {
    save: bool,
    close: bool,
    delete: bool,
    generate: bool,
    export: bool,
    import: bool,
}

enum ExportThemeError {
    Cancelled,
    Serialize(serde_json::Error),
    Io(std::io::Error),
}

impl fmt::Display for ExportThemeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => write!(f, "Export cancelled"),
            Self::Serialize(e) => write!(f, "Failed to serialize theme: {e}"),
            Self::Io(e) => write!(f, "Failed to write file: {e}"),
        }
    }
}

impl From<serde_json::Error> for ExportThemeError {
    fn from(e: serde_json::Error) -> Self {
        Self::Serialize(e)
    }
}

impl From<std::io::Error> for ExportThemeError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

fn export_theme(theme: &CustomTheme) -> Result<String, ExportThemeError> {
    let path = rfd::FileDialog::new()
        .set_title("Export Theme")
        .set_file_name(format!("{}.json", theme.name.trim()))
        .add_filter("JSON", &["json"])
        .save_file()
        .ok_or(ExportThemeError::Cancelled)?;
    let json = serde_json::to_string_pretty(theme)?;
    std::fs::write(&path, &json)?;
    Ok(format!("Theme exported to {}", path.display()))
}

fn import_theme() -> Result<CustomTheme, String> {
    let path = rfd::FileDialog::new()
        .set_title("Import Theme")
        .add_filter("JSON", &["json"])
        .pick_file()
        .ok_or_else(|| "No file selected".to_string())?;
    let data = std::fs::read_to_string(&path).map_err(|e| format!("Failed to read file: {e}"))?;
    serde_json::from_str::<CustomTheme>(&data).map_err(|e| format!("Invalid theme file: {e}"))
}

impl DirigentApp {
    /// Render the custom theme editor dialog.
    pub(in crate::app) fn render_custom_theme_dialog(&mut self, ctx: &egui::Context) {
        if self.custom_theme_edit.is_none() {
            return;
        }

        self.poll_custom_theme_ai_result();
        let mut actions = CustomThemeDialogActions::default();

        egui::Window::new("Custom Theme")
            .collapsible(false)
            .resizable(false)
            .default_size([420.0, 0.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.dialog_frame())
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let Some(edit) = self.custom_theme_edit.as_mut() else {
                    return;
                };
                render_custom_theme_contents(ui, edit, &mut actions);
            });

        self.apply_custom_theme_dialog_actions(actions);
    }

    fn poll_custom_theme_ai_result(&mut self) {
        let edit = match self.custom_theme_edit.as_mut() {
            Some(e) => e,
            None => return,
        };
        match edit.ai_op.poll() {
            AsyncPoll::Ready(result) => apply_custom_theme_ai_result(edit, result),
            AsyncPoll::Failed => {
                edit.ai_error = Some("Theme generation ended unexpectedly".to_string());
            }
            AsyncPoll::Idle | AsyncPoll::Pending => {}
        }
    }

    fn apply_custom_theme_dialog_actions(&mut self, actions: CustomThemeDialogActions) {
        self.apply_custom_theme_file_actions(&actions);
        if actions.generate {
            self.spawn_ai_theme_generation();
        }
        self.apply_custom_theme_finish_actions(actions);
    }

    fn apply_custom_theme_file_actions(&mut self, actions: &CustomThemeDialogActions) {
        if actions.export {
            self.export_current_custom_theme();
        }
        if actions.import {
            self.import_custom_theme_file();
        }
    }

    fn export_current_custom_theme(&mut self) {
        let Some(edit) = &self.custom_theme_edit else {
            return;
        };
        match export_theme(&edit.theme) {
            Ok(msg) => self.set_status_message(msg),
            Err(ExportThemeError::Cancelled) => {}
            Err(e) => self.set_status_message(e.to_string()),
        }
    }

    fn import_custom_theme_file(&mut self) {
        match import_theme() {
            Ok(imported) => {
                if let Some(edit) = self.custom_theme_edit.as_mut() {
                    edit.theme = imported;
                }
            }
            Err(e) if e != "No file selected" => {
                self.set_status_message(format!("Import failed: {e}"));
            }
            Err(_) => {}
        }
    }

    fn apply_custom_theme_finish_actions(&mut self, actions: CustomThemeDialogActions) {
        if actions.save {
            cancel_theme_generation(self.custom_theme_edit.as_ref());
            self.save_custom_theme();
        }
        if actions.delete {
            cancel_theme_generation(self.custom_theme_edit.as_ref());
            self.delete_custom_theme();
        }
        if actions.close {
            cancel_theme_generation(self.custom_theme_edit.as_ref());
            self.custom_theme_edit = None;
        }
    }

    /// Save the currently edited custom theme, deduplicating by name.
    fn save_custom_theme(&mut self) {
        let edit = match self.custom_theme_edit.take() {
            Some(e) => e,
            None => return,
        };
        let mut theme = edit.theme;
        theme.name = theme.name.trim().to_string();

        upsert_custom_theme(&mut self.settings.custom_themes, edit.editing_index, &theme);

        self.settings.theme = crate::settings::ThemeChoice::Custom(theme);
        if let Err(e) = crate::settings::save_settings(&self.project_root, &self.settings) {
            self.set_status_message(format!("Failed to save theme: {e}"));
        }
        self.needs_theme_apply = true;
    }

    /// Delete the currently edited custom theme.
    fn delete_custom_theme(&mut self) {
        let edit = match self.custom_theme_edit.take() {
            Some(e) => e,
            None => return,
        };
        let mut removed_theme_info = None;
        if let Some(idx) = edit.editing_index {
            if idx < self.settings.custom_themes.len() {
                let ct = &self.settings.custom_themes[idx];
                removed_theme_info = Some((ct.name.clone(), ct.is_dark));
                self.settings.custom_themes.remove(idx);
            }
        }
        if let Some((name, was_dark)) = removed_theme_info {
            if matches!(&self.settings.theme, crate::settings::ThemeChoice::Custom(ct) if ct.name == name)
            {
                self.settings.theme = if was_dark {
                    crate::settings::ThemeChoice::Dark
                } else {
                    crate::settings::ThemeChoice::Light
                };
            }
        }
        if let Err(e) = crate::settings::save_settings(&self.project_root, &self.settings) {
            self.set_status_message(format!("Failed to save settings: {e}"));
        }
        self.needs_theme_apply = true;
    }

    /// Spawn a background thread to generate theme colors via Claude Code CLI.
    fn spawn_ai_theme_generation(&mut self) {
        let edit = match self.custom_theme_edit.as_mut() {
            Some(e) => e,
            None => return,
        };
        let prompt_text = edit.ai_prompt.trim().to_string();
        if prompt_text.is_empty() {
            return;
        }

        let tx = match edit.ai_op.arm() {
            Some(tx) => tx,
            None => return,
        };
        edit.ai_error = None;
        edit.ai_cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

        let is_dark = edit.theme.is_dark;
        let current_theme = edit.theme.clone();
        let provider = self.settings.cli_provider.clone();
        let settings = self.settings.clone();
        let project_root = self.project_root.clone();
        let cancel = std::sync::Arc::clone(&edit.ai_cancel);
        let ctx = self.egui_ctx.clone();

        std::thread::spawn(move || {
            let result = generate_theme_via_cli(
                &provider,
                &settings,
                &project_root,
                &prompt_text,
                is_dark,
                &current_theme,
                cancel.clone(),
            );
            let _ = tx.send(result);
            if let Some(c) = ctx.get() {
                c.request_repaint();
            }
        });
    }
}

fn apply_custom_theme_ai_result(edit: &mut CustomThemeEdit, result: Result<CustomTheme, String>) {
    match result {
        Ok(generated) => {
            let name = edit.theme.name.clone();
            let is_dark = edit.theme.is_dark;
            edit.theme = generated;
            edit.theme.name = name;
            edit.theme.is_dark = is_dark;
            edit.ai_error = None;
        }
        Err(e) => {
            edit.ai_error = Some(e);
        }
    }
}

fn render_custom_theme_contents(
    ui: &mut egui::Ui,
    edit: &mut CustomThemeEdit,
    actions: &mut CustomThemeDialogActions,
) {
    render_custom_theme_name_grid(ui, edit);
    section_heading(ui, "Colors");
    render_custom_theme_colors(ui, &mut edit.theme);
    section_heading(ui, "Generate with AI");
    render_custom_theme_ai(ui, edit, actions);
    section_heading(ui, "Import / Export");
    render_custom_theme_import_export(ui, actions);
    ui.add_space(SPACE_MD);
    render_custom_theme_buttons(ui, edit, actions);
}

fn section_heading(ui: &mut egui::Ui, heading: &str) {
    ui.add_space(SPACE_SM);
    ui.separator();
    ui.add_space(SPACE_XS);
    ui.strong(heading);
    ui.add_space(SPACE_XS);
}

fn render_custom_theme_name_grid(ui: &mut egui::Ui, edit: &mut CustomThemeEdit) {
    egui::Grid::new("custom_theme_name_grid")
        .num_columns(2)
        .spacing([SPACE_MD, SPACE_SM])
        .show(ui, |ui| {
            ui.label("Name:");
            ui.add(
                egui::TextEdit::singleline(&mut edit.theme.name)
                    .desired_width(280.0)
                    .hint_text("My Custom Theme"),
            );
            ui.end_row();

            ui.label("Type:");
            egui::ComboBox::from_id_salt("custom_theme_type")
                .selected_text(if edit.theme.is_dark { "Dark" } else { "Light" })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut edit.theme.is_dark, true, "Dark");
                    ui.selectable_value(&mut edit.theme.is_dark, false, "Light");
                });
            ui.end_row();
        });
}

fn render_custom_theme_colors(ui: &mut egui::Ui, theme: &mut CustomTheme) {
    egui::Grid::new("custom_theme_colors_grid")
        .num_columns(4)
        .spacing([SPACE_SM, SPACE_XS])
        .show(ui, |ui| {
            color_row(
                ui,
                "Panel Fill",
                &mut theme.panel_fill,
                "Window Fill",
                &mut theme.window_fill,
            );
            color_row(
                ui,
                "Extreme BG",
                &mut theme.extreme_bg,
                "Faint BG",
                &mut theme.faint_bg,
            );
            color_row(
                ui,
                "Text",
                &mut theme.text,
                "Selection",
                &mut theme.selection,
            );
            color_row(
                ui,
                "Non-interactive",
                &mut theme.noninteractive,
                "Inactive",
                &mut theme.inactive,
            );
            color_row(
                ui,
                "Hovered",
                &mut theme.hovered,
                "Active",
                &mut theme.active,
            );
            color_row(
                ui,
                "Hyperlink",
                &mut theme.hyperlink,
                "Accent",
                &mut theme.accent,
            );
            color_row(
                ui,
                "Warning/Badge",
                &mut theme.warning,
                "Badge Text",
                &mut theme.badge_text,
            );
        });
}

fn render_custom_theme_ai(
    ui: &mut egui::Ui,
    edit: &mut CustomThemeEdit,
    actions: &mut CustomThemeDialogActions,
) {
    ui.horizontal(|ui| {
        let resp = ui.add(
            egui::TextEdit::singleline(&mut edit.ai_prompt)
                .desired_width(320.0)
                .hint_text("e.g. ocean sunset, warm earth tones, cyberpunk neon..."),
        );
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let can_generate = !edit.ai_op.in_flight() && !edit.ai_prompt.trim().is_empty();
        if edit.ai_op.in_flight() {
            ui.spinner();
        } else if ui
            .add_enabled(can_generate, egui::Button::new("Generate"))
            .clicked()
            || enter && can_generate
        {
            actions.generate = true;
        }
    });

    if let Some(err) = &edit.ai_error {
        ui.colored_label(egui::Color32::from_rgb(210, 95, 95), err);
    }
}

fn render_custom_theme_import_export(ui: &mut egui::Ui, actions: &mut CustomThemeDialogActions) {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Export").clicked() {
                actions.export = true;
            }
            if ui.button("Import").clicked() {
                actions.import = true;
            }
        });
    });
}

fn render_custom_theme_buttons(
    ui: &mut egui::Ui,
    edit: &CustomThemeEdit,
    actions: &mut CustomThemeDialogActions,
) {
    let name_valid = !edit.theme.name.trim().is_empty();
    ui.horizontal(|ui| {
        if edit.editing_index.is_some()
            && ui
                .button(egui::RichText::new("Delete").color(egui::Color32::from_rgb(210, 95, 95)))
                .clicked()
        {
            actions.delete = true;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled(name_valid, egui::Button::new("Save"))
                .clicked()
            {
                actions.save = true;
            }
            if ui.button("Cancel").clicked() {
                actions.close = true;
            }
        });
    });
}

fn cancel_theme_generation(edit: Option<&CustomThemeEdit>) {
    if let Some(edit) = edit {
        edit.ai_cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

fn upsert_custom_theme(
    themes: &mut Vec<CustomTheme>,
    editing_index: Option<usize>,
    theme: &CustomTheme,
) {
    let dup_idx = themes
        .iter()
        .position(|t| t.name.trim().eq_ignore_ascii_case(theme.name.trim()));

    match (editing_index, dup_idx) {
        (Some(idx), Some(dup)) if dup == idx && idx < themes.len() => {
            themes[idx] = theme.clone();
        }
        (Some(idx), Some(dup)) => {
            themes[dup] = theme.clone();
            if idx < themes.len() {
                themes.remove(idx);
            }
        }
        (Some(idx), None) if idx < themes.len() => {
            themes[idx] = theme.clone();
        }
        (None, Some(dup)) => {
            themes[dup] = theme.clone();
        }
        _ => {
            themes.push(theme.clone());
        }
    }
}

/// Render a row of two labeled color pickers.
fn color_row(
    ui: &mut egui::Ui,
    label1: &str,
    color1: &mut [u8; 3],
    label2: &str,
    color2: &mut [u8; 3],
) {
    ui.label(label1);
    ui.color_edit_button_srgb(color1);
    ui.label(label2);
    ui.color_edit_button_srgb(color2);
    ui.end_row();
}

/// Format the theme's current colors as a `field: [R, G, B]` list for the AI prompt,
/// so the model can tweak specific colors instead of always regenerating from scratch.
fn current_theme_colors(t: &CustomTheme) -> String {
    let fields: [(&str, [u8; 3]); 14] = [
        ("panel_fill", t.panel_fill),
        ("window_fill", t.window_fill),
        ("extreme_bg", t.extreme_bg),
        ("faint_bg", t.faint_bg),
        ("text", t.text),
        ("selection", t.selection),
        ("noninteractive", t.noninteractive),
        ("inactive", t.inactive),
        ("hovered", t.hovered),
        ("active", t.active),
        ("hyperlink", t.hyperlink),
        ("accent", t.accent),
        ("warning", t.warning),
        ("badge_text", t.badge_text),
    ];
    fields
        .iter()
        .map(|(name, [r, g, b])| format!("  {name}: [{r}, {g}, {b}]"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Call the selected code generator CLI to generate theme palette colors from a description.
fn generate_theme_via_cli(
    provider: &CliProvider,
    settings: &crate::settings::Settings,
    project_root: &std::path::Path,
    description: &str,
    is_dark: bool,
    current: &CustomTheme,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<CustomTheme, String> {
    let dark_light = if is_dark { "dark" } else { "light" };
    let current_colors = current_theme_colors(current);
    let prompt = format!(
        r#"Generate a {dark_light} color theme for a code editor based on this description: "{description}"

The theme currently has these colors (field: [R, G, B]):

{current_colors}

Treat the description as a request to modify this existing palette. If it names a
specific color or aspect (e.g. "make the accent more blue", "warmer background",
"only change the warning color"), keep every other field at its current value and
adjust only what the description asks for. If the description calls for a complete
new look, you may change all of them.

Return ONLY a JSON object (no markdown, no explanation) with these exact fields, each being an array of 3 integers [R, G, B] (0-255):

{{
  "panel_fill": [R, G, B],
  "window_fill": [R, G, B],
  "extreme_bg": [R, G, B],
  "faint_bg": [R, G, B],
  "text": [R, G, B],
  "selection": [R, G, B],
  "noninteractive": [R, G, B],
  "inactive": [R, G, B],
  "hovered": [R, G, B],
  "active": [R, G, B],
  "hyperlink": [R, G, B],
  "accent": [R, G, B],
  "warning": [R, G, B],
  "badge_text": [R, G, B]
}}

For a {dark_light} theme:
- panel_fill, window_fill, extreme_bg, faint_bg should be {bg_desc}
- text should be {text_desc}
- selection, noninteractive, inactive, hovered should be subtle variations of the background
- active, hyperlink, accent should be vibrant accent colors matching the description
- warning should be an attention color (used for badges like the non-default branch indicator)
- badge_text should be a readable text color drawn on top of the warning/accent badge backgrounds

Return ONLY the JSON object."#,
        bg_desc = if is_dark {
            "dark backgrounds (RGB values roughly 20-70)"
        } else {
            "light backgrounds (RGB values roughly 220-255)"
        },
        text_desc = if is_dark {
            "light (RGB values roughly 180-240)"
        } else {
            "dark (RGB values roughly 20-80)"
        },
    );

    let response_text =
        crate::app::split_cue::run_cli_prompt(&prompt, provider, project_root, settings, cancel)?;

    parse_theme_json(&response_text, is_dark)
}

/// Parse the JSON response from Claude into a CustomTheme.
fn parse_theme_json(text: &str, is_dark: bool) -> Result<CustomTheme, String> {
    let json_str = crate::util::json_extract::extract_json(text);
    if !json_str.starts_with('{') {
        return Err("No JSON object found in AI response".to_string());
    }

    let parsed: serde_json::Value =
        serde_json::from_str(&json_str).map_err(|e| format!("Failed to parse JSON: {e}"))?;

    let get_rgb = |key: &str| -> Result<[u8; 3], String> {
        let arr = parsed
            .get(key)
            .and_then(|v| v.as_array())
            .ok_or_else(|| format!("Missing or invalid field: {key}"))?;
        if arr.len() != 3 {
            return Err(format!("Field {key} must have exactly 3 values"));
        }
        let mut rgb = [0u8; 3];
        for (i, channel) in ["r", "g", "b"].iter().enumerate() {
            let val = arr[i]
                .as_u64()
                .ok_or_else(|| format!("{key}[{channel}]: expected integer"))?;
            rgb[i] = u8::try_from(val)
                .map_err(|_| format!("{key}[{channel}]: value {val} out of range 0..=255"))?;
        }
        Ok(rgb)
    };

    // Warning / badge text are optional in the AI response: fall back to
    // theme-appropriate defaults if the model omits them.
    let (default_warning, default_badge_text) = if is_dark {
        ([200u8, 165, 60], [220u8, 220, 220])
    } else {
        ([160u8, 110, 0], [255u8, 255, 255])
    };

    Ok(CustomTheme {
        name: String::new(),
        is_dark,
        panel_fill: get_rgb("panel_fill")?,
        window_fill: get_rgb("window_fill")?,
        extreme_bg: get_rgb("extreme_bg")?,
        faint_bg: get_rgb("faint_bg")?,
        text: get_rgb("text")?,
        selection: get_rgb("selection")?,
        noninteractive: get_rgb("noninteractive")?,
        inactive: get_rgb("inactive")?,
        hovered: get_rgb("hovered")?,
        active: get_rgb("active")?,
        hyperlink: get_rgb("hyperlink")?,
        accent: get_rgb("accent")?,
        warning: get_rgb("warning").unwrap_or(default_warning),
        badge_text: get_rgb("badge_text").unwrap_or(default_badge_text),
    })
}
