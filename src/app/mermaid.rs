//! Mermaid diagram rendering for the Markdown viewer.
//!
//! Fenced code blocks tagged ```` ```mermaid ```` are rendered to images by
//! shelling out to the external [`merman-cli`](https://crates.io/crates/merman-cli)
//! tool (a browserless, native Mermaid renderer). Rendering happens on a
//! background thread; the resulting PNG is decoded into an egui texture and
//! cached, keyed by the diagram source plus the active light/dark theme.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::mpsc;

use eframe::egui;

/// Name of the external CLI used to render Mermaid diagrams.
const MERMAN_BIN: &str = "merman-cli";

/// State for the enlarged Mermaid diagram viewer dialog.
pub(super) struct MermaidDialog {
    /// The diagram source (used to look up the texture and re-render for export).
    pub source: String,
    /// Theme the diagram was rendered with (light/dark).
    pub dark: bool,
    /// Current zoom factor (1.0 = native pixel size).
    pub zoom: f32,
}

impl MermaidDialog {
    pub(super) fn new(source: String, dark: bool) -> Self {
        Self {
            source,
            dark,
            zoom: 1.0,
        }
    }
}

/// Per-diagram render state.
pub(super) enum MermaidState {
    /// A background render is in progress.
    Loading,
    /// The diagram rendered successfully into a texture.
    Ready(egui::TextureHandle),
    /// Rendering failed (CLI missing, parse error, …). Holds a message.
    Error(String),
}

/// Cache of rendered Mermaid diagrams plus the channel used to receive
/// background render results.
pub(super) struct MermaidCache {
    entries: HashMap<u64, MermaidState>,
    tx: mpsc::Sender<(u64, Result<egui::ColorImage, String>)>,
    rx: mpsc::Receiver<(u64, Result<egui::ColorImage, String>)>,
    /// Lazily-checked availability of the `merman-cli` binary.
    available: Option<bool>,
}

impl Default for MermaidCache {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            entries: HashMap::new(),
            tx,
            rx,
            available: None,
        }
    }
}

/// Hash a diagram source together with the active theme so a light/dark switch
/// produces a fresh render.
fn diagram_key(source: &str, dark: bool) -> u64 {
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    dark.hash(&mut hasher);
    hasher.finish()
}

impl MermaidCache {
    /// Drain finished background renders, turning decoded images into textures.
    pub(super) fn poll(&mut self, ctx: &egui::Context) {
        while let Ok((key, result)) = self.rx.try_recv() {
            let state = match result {
                Ok(image) => {
                    let name = format!("mermaid-{key:016x}");
                    let texture = ctx.load_texture(name, image, egui::TextureOptions::LINEAR);
                    MermaidState::Ready(texture)
                }
                Err(message) => MermaidState::Error(message),
            };
            self.entries.insert(key, state);
        }
    }

    /// Ensure a render exists (or is in flight) for the given diagram. Spawns a
    /// background thread on first request.
    pub(super) fn ensure(&mut self, source: &str, dark: bool, ctx: &egui::Context) {
        let key = diagram_key(source, dark);
        if self.entries.contains_key(&key) {
            return;
        }

        // Resolve the binary once; if it is missing, record an error so the
        // viewer can fall back to showing the raw diagram source.
        let available = *self
            .available
            .get_or_insert_with(|| which::which(MERMAN_BIN).is_ok());
        if !available {
            self.entries.insert(
                key,
                MermaidState::Error(format!(
                    "`{MERMAN_BIN}` not found in PATH — install it to render Mermaid diagrams."
                )),
            );
            return;
        }

        self.entries.insert(key, MermaidState::Loading);

        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let source = source.to_string();
        std::thread::spawn(move || {
            let result = render_color_image(&source, dark);
            // Ignore send errors: the app may have closed.
            let _ = tx.send((key, result));
            ctx.request_repaint();
        });
    }

    /// Look up the render state for a diagram, if any.
    pub(super) fn get(&self, source: &str, dark: bool) -> Option<&MermaidState> {
        self.entries.get(&diagram_key(source, dark))
    }
}

/// Render Mermaid `source` to a PNG via `merman-cli` and decode it into an
/// egui [`ColorImage`]. Runs on a background thread.
fn render_color_image(source: &str, dark: bool) -> Result<egui::ColorImage, String> {
    let png = render_to_bytes(source, dark, "png")?;
    let decoded = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
        .map_err(|e| format!("failed to decode rendered diagram: {e}"))?;
    let rgba = decoded.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        size,
        rgba.as_flat_samples().samples,
    ))
}

/// Invoke `merman-cli` to render the diagram source to bytes in the format
/// implied by `ext` (e.g. `"png"` or `"svg"`).
///
/// Writes the source and reads the output through a temp directory rather than
/// stdin/stdout so the output format is unambiguously inferred from the file
/// extension.
pub(super) fn render_to_bytes(source: &str, dark: bool, ext: &str) -> Result<Vec<u8>, String> {
    let bin = which::which(MERMAN_BIN).map_err(|_| format!("`{MERMAN_BIN}` not found in PATH"))?;
    let dir = tempfile::tempdir().map_err(|e| format!("could not create temp dir: {e}"))?;
    let input = dir.path().join("diagram.mmd");
    let output = dir.path().join(format!("diagram.{ext}"));
    std::fs::write(&input, source).map_err(|e| format!("could not write diagram source: {e}"))?;

    let theme = if dark { "dark" } else { "default" };
    let result = std::process::Command::new(&bin)
        .arg("-i")
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .arg("-t")
        .arg(theme)
        .arg("-b")
        .arg("transparent")
        .output()
        .map_err(|e| format!("failed to run {MERMAN_BIN}: {e}"))?;

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        let message = stderr.trim();
        return Err(if message.is_empty() {
            format!("{MERMAN_BIN} exited with an error")
        } else {
            message.to_string()
        });
    }

    std::fs::read(&output).map_err(|e| format!("could not read rendered diagram: {e}"))
}
