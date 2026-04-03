#![allow(unexpected_cfgs)]

mod agents;
mod app;
mod claude;
mod db;
mod diff_view;
mod error;
mod file_tree;
mod git;
mod lsp;
mod opencode;
mod prompt_hints;
mod prompt_suggestions;
mod settings;
mod sources;
mod syntax;
mod telemetry;

use eframe::egui;
use std::path::PathBuf;

/// Launch a new instance of the Dirigent app bundle (via `open -n`).
/// Falls back to spawning the raw binary when not running from a bundle.
pub fn spawn_new_instance() {
    if let Ok(exe) = std::env::current_exe() {
        let exe_str = exe.to_string_lossy().to_string();
        if let Some(app_pos) = exe_str.find(".app/") {
            let bundle_path = &exe_str[..app_pos + 4];
            let _ = std::process::Command::new("open")
                .arg("-n")
                .arg(bundle_path)
                .spawn();
        } else {
            // Running from terminal — just launch the binary again
            let _ = std::process::Command::new(&exe).spawn();
        }
    }
}

#[cfg(target_os = "macos")]
/// Walk the main menu to find the About item and retarget it to our helper.
///
/// # Safety
/// Caller must pass valid ObjC pointers obtained from NSApplication.
unsafe fn retarget_about_menu_item(
    app: *mut objc::runtime::Object,
    helper: *mut objc::runtime::Object,
) {
    use objc::runtime::Object;
    use objc::{msg_send, sel, sel_impl};

    let main_menu: *mut Object = msg_send![app, mainMenu];
    if main_menu.is_null() {
        return;
    }
    let count: isize = msg_send![main_menu, numberOfItems];
    if count == 0 {
        return;
    }
    let app_menu_item: *mut Object = msg_send![main_menu, itemAtIndex:0_isize];
    let submenu: *mut Object = msg_send![app_menu_item, submenu];
    if submenu.is_null() {
        return;
    }
    let sub_count: isize = msg_send![submenu, numberOfItems];
    let about_sel = sel!(orderFrontStandardAboutPanel:);
    for i in 0..sub_count {
        let item: *mut Object = msg_send![submenu, itemAtIndex:i];
        let action: objc::runtime::Sel = msg_send![item, action];
        if action == about_sel {
            let _: () = msg_send![item, setTarget:helper];
            let _: () = msg_send![item, setAction:sel!(showAbout:)];
            break;
        }
    }
}

#[cfg(target_os = "macos")]
/// Create an NSImage from raw PNG bytes.
///
/// # Safety
/// Caller must ensure ObjC runtime is available (i.e. running on macOS).
unsafe fn nsimage_from_png(png_bytes: &[u8]) -> *mut objc::runtime::Object {
    use objc::runtime::{Class, Object};
    use objc::{msg_send, sel, sel_impl};

    let ns_data = Class::get("NSData").unwrap();
    let data: *mut Object =
        msg_send![ns_data, dataWithBytes:png_bytes.as_ptr() length:png_bytes.len()];
    let ns_image = Class::get("NSImage").unwrap();
    let image: *mut Object = msg_send![ns_image, alloc];
    msg_send![image, initWithData:data]
}

#[cfg(target_os = "macos")]
fn setup_macos_about_panel() {
    use objc::declare::ClassDecl;
    use objc::runtime::{Class, Object, Sel};
    use objc::{msg_send, sel, sel_impl};

    unsafe {
        let ns_app = Class::get("NSApplication").unwrap();
        let app: *mut Object = msg_send![ns_app, sharedApplication];

        // Set application icon (used by dock and About panel)
        let image = nsimage_from_png(include_bytes!("../assets/logo.png"));
        let _: () = msg_send![app, setApplicationIconImage:image];

        // Create a helper class whose showAbout: method opens the standard
        // About panel with our name and version filled in.
        let superclass = Class::get("NSObject").unwrap();
        if let Some(mut decl) = ClassDecl::new("DirigentAboutHelper", superclass) {
            extern "C" fn show_about(_this: &Object, _sel: Sel, _sender: *mut Object) {
                unsafe {
                    let ns_app = Class::get("NSApplication").unwrap();
                    let app: *mut Object = msg_send![ns_app, sharedApplication];
                    let ns_dict = Class::get("NSMutableDictionary").unwrap();
                    let dict: *mut Object = msg_send![ns_dict, new];
                    let ns_string = Class::get("NSString").unwrap();

                    let key: *mut Object = msg_send![ns_string,
                        stringWithUTF8String: c"ApplicationName".as_ptr()];
                    let val: *mut Object = msg_send![ns_string,
                        stringWithUTF8String: c"Dirigent".as_ptr()];
                    let _: () = msg_send![dict, setObject:val forKey:key];

                    let key: *mut Object = msg_send![ns_string,
                        stringWithUTF8String: c"ApplicationVersion".as_ptr()];
                    let val: *mut Object = msg_send![ns_string,
                        stringWithUTF8String: concat!(env!("BUILD_VERSION"), "\0").as_ptr()];
                    let _: () = msg_send![dict, setObject:val forKey:key];

                    // Pass our icon so the About panel shows it instead of the
                    // generic macOS folder icon.
                    let icon = nsimage_from_png(include_bytes!("../assets/logo.png"));
                    let key: *mut Object = msg_send![ns_string,
                        stringWithUTF8String: c"ApplicationIcon".as_ptr()];
                    let _: () = msg_send![dict, setObject:icon forKey:key];

                    let _: () = msg_send![app, orderFrontStandardAboutPanelWithOptions:dict];
                }
            }

            decl.add_method(
                sel!(showAbout:),
                show_about as extern "C" fn(&Object, Sel, *mut Object),
            );

            let helper_class = decl.register();
            let helper: *mut Object = msg_send![helper_class, new];

            // Find the native About menu item and retarget it to our helper
            retarget_about_menu_item(app, helper);

            let _ = helper; // prevent deallocation
        }
    }
}

/// Compute a centered window position using NSScreen, so the window is created
/// at the right spot and macOS does not emit "Window move completed without
/// beginning" when winit repositions it after creation.
#[cfg(target_os = "macos")]
fn screen_center_position(win_width: f32, win_height: f32) -> Option<egui::Pos2> {
    #[repr(C)]
    #[derive(Copy, Clone)]
    struct CGSize {
        width: f64,
        height: f64,
    }
    #[repr(C)]
    #[derive(Copy, Clone)]
    struct CGPoint {
        x: f64,
        y: f64,
    }
    #[repr(C)]
    #[derive(Copy, Clone)]
    struct CGRect {
        origin: CGPoint,
        size: CGSize,
    }

    unsafe {
        use objc::runtime::{Class, Object};
        use objc::{msg_send, sel, sel_impl};

        let ns_screen = Class::get("NSScreen")?;
        let main_screen: *mut Object = msg_send![ns_screen, mainScreen];
        if main_screen.is_null() {
            return None;
        }

        let frame: CGRect = msg_send![main_screen, frame];
        let x = ((frame.size.width as f32) - win_width) / 2.0;
        let y = ((frame.size.height as f32) - win_height) / 2.0;

        Some(egui::pos2(x.max(0.0), y.max(0.0)))
    }
}

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
    telemetry::init();

    let sentry_dsn = std::env::var("SENTRY_DSN")
        .ok()
        .or_else(|| {
            claude::load_env_var_with_dirigent_fallback(
                &std::env::current_dir().unwrap_or_default(),
                "SENTRY_DSN",
            )
        })
        .unwrap_or_default();
    let _sentry_guard = sentry::init((
        sentry_dsn,
        sentry::ClientOptions {
            release: sentry::release_name!(),
            send_default_pii: true,
            ..Default::default()
        },
    ));

    // Filter out macOS Process Serial Number args (passed by Finder/Launch Services)
    let args: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with("-psn"))
        .collect();

    let explicit_path = args.first().map(|arg| app::util::expand_tilde(arg));

    // Detect Finder launch: no explicit path and running from inside an .app bundle
    let launched_from_app_bundle = explicit_path.is_none()
        && std::env::current_exe()
            .map(|p| p.to_string_lossy().contains(".app/Contents/MacOS/"))
            .unwrap_or(false);

    let project_root = if let Some(path) = explicit_path {
        path
    } else {
        std::env::current_dir().expect("failed to get cwd")
    };

    let project_root = std::fs::canonicalize(&project_root).unwrap_or(project_root);

    // When launched from Finder, use the home directory as a temporary root
    // and auto-show the repo picker so the user can choose a project.
    let (project_root, show_repo_picker) = if launched_from_app_bundle {
        let home = std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or(project_root);
        (home, true)
    } else {
        // Launched with an explicit project — remember it globally.
        settings::add_global_recent_project(&project_root.to_string_lossy());
        (project_root, false)
    };

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1200.0, 800.0])
        .with_title(format!(
            "Dirigent - {}",
            project_root
                .file_name()
                .map(|n| n.to_string_lossy())
                .unwrap_or_else(|| project_root.to_string_lossy())
        ))
        .with_icon(std::sync::Arc::new(load_logo_icon()));

    #[cfg(target_os = "macos")]
    if let Some(pos) = screen_center_position(1200.0, 800.0) {
        viewport = viewport.with_position(pos);
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    // Use a unique app ID so each launch creates a new instance rather than
    // activating an existing one (which is macOS default behavior for apps
    // with the same bundle identifier).
    let unique_id = uuid::Uuid::new_v4().to_string()[..8].to_string();
    let app_id = format!("Dirigent-{}", unique_id);

    eframe::run_native(
        &app_id,
        options,
        Box::new(move |cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            cc.egui_ctx.set_visuals(egui::Visuals::dark());

            // Pre-register the "Icons" font family so the first frame never
            // panics with "FontFamily::Name(\"Icons\") is not bound to any
            // fonts".  apply_theme() will overwrite this with the real setup.
            {
                let mut fd = egui::FontDefinitions::default();
                let mono = fd
                    .families
                    .get(&egui::FontFamily::Monospace)
                    .cloned()
                    .unwrap_or_default();
                fd.families
                    .insert(egui::FontFamily::Name("Icons".into()), mono);
                cc.egui_ctx.set_fonts(fd);
            }

            #[cfg(target_os = "macos")]
            setup_macos_about_panel();

            let project_name = project_root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let mut app = app::DirigentApp::new(project_root, show_repo_picker);
            if show_repo_picker {
                app.show_repo_picker = true;
            }
            telemetry::emit_app_started(&project_name);
            Ok(Box::new(app))
        }),
    )
}
