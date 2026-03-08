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
    // Load SVG file
    let svg_bytes = include_bytes!("../assets/logo.svg");
    
    // For now, create a placeholder icon since SVG rasterization requires additional dependencies
    // TODO: Add resvg or similar crate to properly rasterize SVG to RGBA
    let size = 64u32;
    let mut rgba = vec![0u8; (size * size * 4) as usize];

    // Create a simple colored square as placeholder until SVG rasterization is implemented
    for y in 0..size {
        for x in 0..size {
            let idx = ((y * size + x) * 4) as usize;
            rgba[idx] = 100;     // R
            rgba[idx + 1] = 150; // G  
            rgba[idx + 2] = 200; // B
            rgba[idx + 3] = 255;
        }
    }

    egui::IconData { rgba, width: size, height: size }
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
