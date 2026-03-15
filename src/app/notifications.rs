/// Send a macOS notification via `UNUserNotificationCenter` (modern API).
/// Registers a delegate on first call so notifications are shown even when the
/// app is in the foreground (macOS suppresses them by default).
/// Falls back to `osascript display notification` when running without a bundle.
#[cfg(target_os = "macos")]
pub(super) fn send_macos_notification(title: &str, subtitle: &str, body: &str) {
    use objc::declare::ClassDecl;
    use objc::runtime::{Class, Object, Sel};
    use objc::{msg_send, sel, sel_impl};
    use std::ffi::CString;
    use std::sync::Once;

    // ── Objective-C block helpers ──

    /// Layout for a `void (^)(BOOL, NSError *)` authorization completion block.
    #[repr(C)]
    struct AuthBlock {
        isa: *const std::ffi::c_void,
        flags: i32,
        reserved: i32,
        invoke: unsafe extern "C" fn(*mut AuthBlock, u8, *mut Object),
        descriptor: *const BlockDesc,
    }

    #[repr(C)]
    struct BlockDesc {
        reserved: usize,
        size: usize,
    }

    extern "C" {
        static _NSConcreteStackBlock: std::ffi::c_void;
    }

    unsafe extern "C" fn noop_auth(_b: *mut AuthBlock, _granted: u8, _err: *mut Object) {}

    static AUTH_DESC: BlockDesc = BlockDesc {
        reserved: 0,
        size: std::mem::size_of::<AuthBlock>(),
    };

    /// Minimal layout of an ObjC block received as a callback parameter —
    /// just enough to read and call the `invoke` function pointer.
    #[repr(C)]
    struct ReceivedBlock {
        _isa: usize,
        _flags: i32,
        _reserved: i32,
        invoke: unsafe extern "C" fn(*const std::ffi::c_void, usize),
    }

    // ── One-time delegate + authorization setup ──

    static SETUP_ONCE: Once = Once::new();

    unsafe {
        let pool_cls = Class::get("NSAutoreleasePool").unwrap();
        let pool: *mut Object = msg_send![pool_cls, new];

        let nsstring = Class::get("NSString").unwrap();

        // Strip null bytes to prevent CString::new panics.
        let title_safe = title.replace('\0', "");
        let subtitle_safe = subtitle.replace('\0', "");
        let body_safe = body.replace('\0', "");

        let title_c = CString::new(title_safe.as_str()).unwrap();
        let sub_c = CString::new(subtitle_safe.as_str()).unwrap();
        let body_c = CString::new(body_safe.as_str()).unwrap();

        let title_ns: *mut Object = msg_send![nsstring, stringWithUTF8String: title_c.as_ptr()];
        let sub_ns: *mut Object = msg_send![nsstring, stringWithUTF8String: sub_c.as_ptr()];
        let body_ns: *mut Object = msg_send![nsstring, stringWithUTF8String: body_c.as_ptr()];

        let mut delivered = false;

        // ── Modern API: UNUserNotificationCenter (macOS 10.14+) ──

        let bundle_cls = Class::get("NSBundle").unwrap();
        let fw_path_c =
            CString::new("/System/Library/Frameworks/UserNotifications.framework").unwrap();
        let fw_path_ns: *mut Object = msg_send![nsstring, stringWithUTF8String: fw_path_c.as_ptr()];
        let fw_bundle: *mut Object = msg_send![bundle_cls, bundleWithPath: fw_path_ns];

        let main_bundle: *mut Object = msg_send![bundle_cls, mainBundle];
        let bundle_id: *mut Object = msg_send![main_bundle, bundleIdentifier];

        if !fw_bundle.is_null() && !bundle_id.is_null() {
            let loaded: bool = msg_send![fw_bundle, load];
            if loaded {
                if let Some(center_cls) = Class::get("UNUserNotificationCenter") {
                    let center: *mut Object = msg_send![center_cls, currentNotificationCenter];
                    if !center.is_null() {
                        // First call: register a delegate that tells macOS to show
                        // notifications even when Dirigent is the active app, and
                        // request notification authorization.
                        SETUP_ONCE.call_once(|| {
                            // ── Delegate ──
                            // Implements willPresentNotification:withCompletionHandler:
                            // to enable foreground banner + sound delivery.
                            extern "C" fn will_present(
                                _this: &Object,
                                _sel: Sel,
                                _center: *mut Object,
                                _notification: *mut Object,
                                handler: *const std::ffi::c_void,
                            ) {
                                unsafe {
                                    let bh = handler as *const ReceivedBlock;
                                    // UNNotificationPresentationOptionBanner  (1 << 4)
                                    // UNNotificationPresentationOptionList    (1 << 3)
                                    // UNNotificationPresentationOptionSound   (1 << 1)
                                    let opts: usize = (1 << 4) | (1 << 3) | (1 << 1);
                                    ((*bh).invoke)(handler, opts);
                                }
                            }

                            // Handle notification click: just call the completion
                            // handler without opening anything (avoids Script Editor).
                            extern "C" fn did_receive_response(
                                _this: &Object,
                                _sel: Sel,
                                _center: *mut Object,
                                _response: *mut Object,
                                handler: *const std::ffi::c_void,
                            ) {
                                unsafe {
                                    let bh = handler as *const ReceivedBlock;
                                    ((*bh).invoke)(handler, 0);
                                }
                            }

                            let superclass = Class::get("NSObject").unwrap();
                            if let Some(mut decl) =
                                ClassDecl::new("DirigentNotifDelegate", superclass)
                            {
                                decl.add_method(
                                    sel!(userNotificationCenter:willPresentNotification:withCompletionHandler:),
                                    will_present
                                        as extern "C" fn(
                                            &Object,
                                            Sel,
                                            *mut Object,
                                            *mut Object,
                                            *const std::ffi::c_void,
                                        ),
                                );
                                decl.add_method(
                                    sel!(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:),
                                    did_receive_response
                                        as extern "C" fn(
                                            &Object,
                                            Sel,
                                            *mut Object,
                                            *mut Object,
                                            *const std::ffi::c_void,
                                        ),
                                );
                                let delegate_cls = decl.register();
                                let delegate: *mut Object = msg_send![delegate_cls, new];

                                if let Some(un_cls) = Class::get("UNUserNotificationCenter") {
                                    let c: *mut Object =
                                        msg_send![un_cls, currentNotificationCenter];
                                    if !c.is_null() {
                                        let _: () = msg_send![c, setDelegate: delegate];
                                    }
                                }
                                // Intentionally leaked — the delegate must outlive the app.
                            }

                            // ── Authorization ──
                            if let Some(un_cls) = Class::get("UNUserNotificationCenter") {
                                let c: *mut Object =
                                    msg_send![un_cls, currentNotificationCenter];
                                if !c.is_null() {
                                    let auth_block = AuthBlock {
                                        isa: &_NSConcreteStackBlock as *const _
                                            as *const std::ffi::c_void,
                                        flags: 0,
                                        reserved: 0,
                                        invoke: noop_auth,
                                        descriptor: &AUTH_DESC,
                                    };
                                    // UNAuthorizationOptionAlert (1<<2) | Sound (1<<1)
                                    let opts: usize = 4 | 2;
                                    let _: () = msg_send![c,
                                        requestAuthorizationWithOptions:opts
                                        completionHandler:&auth_block
                                            as *const _ as *const std::ffi::c_void];
                                }
                            }
                        });

                        // Build notification content.
                        if let Some(content_cls) = Class::get("UNMutableNotificationContent") {
                            let content: *mut Object = msg_send![content_cls, new];
                            let _: () = msg_send![content, setTitle: title_ns];
                            let _: () = msg_send![content, setSubtitle: sub_ns];
                            let _: () = msg_send![content, setBody: body_ns];

                            // Build and deliver the request.
                            if let Some(request_cls) = Class::get("UNNotificationRequest") {
                                let nsuuid_cls = Class::get("NSUUID").unwrap();
                                let uuid: *mut Object = msg_send![nsuuid_cls, UUID];
                                let uuid_str: *mut Object = msg_send![uuid, UUIDString];
                                let trigger: *const Object = std::ptr::null();
                                let request: *mut Object = msg_send![request_cls,
                                    requestWithIdentifier:uuid_str
                                    content:content
                                    trigger:trigger];

                                // completionHandler is nullable.
                                let nil: *const std::ffi::c_void = std::ptr::null();
                                let _: () = msg_send![center,
                                    addNotificationRequest:request
                                    withCompletionHandler:nil];
                                delivered = true;
                            }
                        }
                    }
                }
            }
        }

        // ── Fallback: osascript `display notification` ──
        // Used when running without a bundle identifier (e.g. `cargo run`).
        if !delivered {
            let title_esc = title_safe.replace('\\', "\\\\").replace('"', "\\\"");
            let sub_esc = subtitle_safe.replace('\\', "\\\\").replace('"', "\\\"");
            let body_esc = body_safe.replace('\\', "\\\\").replace('"', "\\\"");
            let script = format!(
                "display notification \"{}\" with title \"{}\" subtitle \"{}\"",
                body_esc, title_esc, sub_esc
            );
            let _ = std::process::Command::new("/usr/bin/osascript")
                .arg("-e")
                .arg(&script)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
            // Not setting `delivered` — best-effort fallback.
        }

        let _: () = msg_send![pool, drain];
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn send_macos_notification(_title: &str, _subtitle: &str, _body: &str) {}
