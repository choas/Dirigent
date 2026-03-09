mod app;
mod claude;
mod db;
mod diff_view;
mod file_tree;
mod git;
mod settings;

use eframe::egui;
use std::path::PathBuf;

#[cfg(target_os = "macos")]
fn setup_macos_about_panel() {
    use objc::declare::ClassDecl;
    use objc::runtime::{Class, Object, Sel};
    use objc::{msg_send, sel, sel_impl};

    unsafe {
        let ns_app = Class::get("NSApplication").unwrap();
        let app: *mut Object = msg_send![ns_app, sharedApplication];

        // Set application icon (used by dock and About panel)
        let png_bytes = include_bytes!("../assets/logo.png");
        let ns_data = Class::get("NSData").unwrap();
        let data: *mut Object =
            msg_send![ns_data, dataWithBytes:png_bytes.as_ptr() length:png_bytes.len()];
        let ns_image = Class::get("NSImage").unwrap();
        let image: *mut Object = msg_send![ns_image, alloc];
        let image: *mut Object = msg_send![image, initWithData:data];
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
                        stringWithUTF8String: b"ApplicationName\0".as_ptr()];
                    let val: *mut Object = msg_send![ns_string,
                        stringWithUTF8String: b"Dirigent\0".as_ptr()];
                    let _: () = msg_send![dict, setObject:val forKey:key];

                    let key: *mut Object = msg_send![ns_string,
                        stringWithUTF8String: b"ApplicationVersion\0".as_ptr()];
                    let val: *mut Object = msg_send![ns_string,
                        stringWithUTF8String: concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr()];
                    let _: () = msg_send![dict, setObject:val forKey:key];

                    // Pass our icon so the About panel shows it instead of the
                    // generic macOS folder icon.
                    let png_bytes = include_bytes!("../assets/logo.png");
                    let ns_data_cls = Class::get("NSData").unwrap();
                    let icon_data: *mut Object = msg_send![ns_data_cls,
                        dataWithBytes:png_bytes.as_ptr() length:png_bytes.len()];
                    let ns_image_cls = Class::get("NSImage").unwrap();
                    let icon: *mut Object = msg_send![ns_image_cls, alloc];
                    let icon: *mut Object = msg_send![icon, initWithData:icon_data];
                    let key: *mut Object = msg_send![ns_string,
                        stringWithUTF8String: b"ApplicationIcon\0".as_ptr()];
                    let _: () = msg_send![dict, setObject:icon forKey:key];

                    let _: () =
                        msg_send![app, orderFrontStandardAboutPanelWithOptions:dict];
                }
            }

            decl.add_method(
                sel!(showAbout:),
                show_about as extern "C" fn(&Object, Sel, *mut Object),
            );

            let helper_class = decl.register();
            let helper: *mut Object = msg_send![helper_class, new];

            // Find the native About menu item and retarget it to our helper
            let main_menu: *mut Object = msg_send![app, mainMenu];
            if !main_menu.is_null() {
                let count: isize = msg_send![main_menu, numberOfItems];
                if count > 0 {
                    let app_menu_item: *mut Object =
                        msg_send![main_menu, itemAtIndex:0_isize];
                    let submenu: *mut Object = msg_send![app_menu_item, submenu];
                    if !submenu.is_null() {
                        let sub_count: isize = msg_send![submenu, numberOfItems];
                        let about_sel = sel!(orderFrontStandardAboutPanel:);
                        for i in 0..sub_count {
                            let item: *mut Object =
                                msg_send![submenu, itemAtIndex:i];
                            let action: Sel = msg_send![item, action];
                            if action == about_sel {
                                let _: () = msg_send![item, setTarget:helper];
                                let _: () =
                                    msg_send![item, setAction:sel!(showAbout:)];
                                break;
                            }
                        }
                    }
                }
            }

            std::mem::forget(helper); // prevent deallocation
        }
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

            #[cfg(target_os = "macos")]
            setup_macos_about_panel();

            Ok(Box::new(app::DirigentApp::new(project_root)))
        }),
    )
}
