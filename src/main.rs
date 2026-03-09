mod app;
mod claude;
mod db;
mod diff_view;
mod file_tree;
mod git;
mod settings;

use eframe::egui;
use std::path::PathBuf;

fn load_logo_icon() -> egui::IconData {
    let png_bytes = include_bytes!("../assets/logo.png");
    let img = image::load_from_memory_with_format(png_bytes, image::ImageFormat::Png)
        .expect("failed to decode logo.png")
        .into_rgba8();
    let (width, height) = img.dimensions();
    egui::IconData {
        rgba: img.into_raw(),
        width,
        height,
    }
}

fn main() -> eframe::Result {
    let project_root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().expect("failed to get cwd"));

    let project_root = std::fs::canonicalize(&project_root)
        .unwrap_or(project_root);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_title(format!("Dirigent - {}", project_root.display()))
            .with_icon(std::sync::Arc::new(load_logo_icon())),
        ..Default::default()
    };

    eframe::run_native(
        "Dirigent",
        options,
        Box::new(move |cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(app::DirigentApp::new(project_root)))
        }),
    )
}
