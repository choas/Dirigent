/// Send a macOS notification via `UNUserNotificationCenter` (modern API).
/// Registers a delegate on first call so notifications are shown even when the
/// app is in the foreground (macOS suppresses them by default).
/// Falls back to `osascript display notification` when running without a bundle.
#[cfg(target_os = "macos")]
pub(super) fn send_macos_notification(title: &str, subtitle: &str, body: &str) {
    use objc::runtime::{Class, Object};
    use objc::{msg_send, sel, sel_impl};
    use std::ffi::CString;

    // Strip null bytes to prevent CString::new panics.
    let title_safe = title.replace('\0', "");
    let subtitle_safe = subtitle.replace('\0', "");
    let body_safe = body.replace('\0', "");

    unsafe {
        let pool_cls = Class::get("NSAutoreleasePool").unwrap();
        let pool: *mut Object = msg_send![pool_cls, new];

        let nsstring = Class::get("NSString").unwrap();

        let title_c = CString::new(title_safe.as_str()).unwrap();
        let sub_c = CString::new(subtitle_safe.as_str()).unwrap();
        let body_c = CString::new(body_safe.as_str()).unwrap();

        let title_ns: *mut Object = msg_send![nsstring, stringWithUTF8String: title_c.as_ptr()];
        let sub_ns: *mut Object = msg_send![nsstring, stringWithUTF8String: sub_c.as_ptr()];
        let body_ns: *mut Object = msg_send![nsstring, stringWithUTF8String: body_c.as_ptr()];

        let delivered = try_modern_notification(title_ns, sub_ns, body_ns);

        // Fallback: osascript `display notification`.
        // Used when running without a bundle identifier (e.g. `cargo run`).
        if !delivered {
            fallback_osascript_notification(&title_safe, &subtitle_safe, &body_safe);
        }

        let _: () = msg_send![pool, drain];
    }
}

/// Try to send a notification via the modern `UNUserNotificationCenter` API
/// (macOS 10.14+). Returns `true` if the notification was successfully
/// scheduled (completion handler reported no error); returns `false` on any
/// failure so the caller can fall back to `osascript`.
#[cfg(target_os = "macos")]
unsafe fn try_modern_notification(
    title_ns: *mut objc::runtime::Object,
    sub_ns: *mut objc::runtime::Object,
    body_ns: *mut objc::runtime::Object,
) -> bool {
    use objc::runtime::{Class, Object};
    use objc::{msg_send, sel, sel_impl};

    let bundle_cls = Class::get("NSBundle").unwrap();
    let nsstring = Class::get("NSString").unwrap();

    let fw_path_c =
        std::ffi::CString::new("/System/Library/Frameworks/UserNotifications.framework").unwrap();
    let fw_path_ns: *mut Object = msg_send![nsstring, stringWithUTF8String: fw_path_c.as_ptr()];
    let fw_bundle: *mut Object = msg_send![bundle_cls, bundleWithPath: fw_path_ns];

    let main_bundle: *mut Object = msg_send![bundle_cls, mainBundle];
    let bundle_id: *mut Object = msg_send![main_bundle, bundleIdentifier];

    if fw_bundle.is_null() || bundle_id.is_null() {
        return false;
    }

    let loaded: bool = msg_send![fw_bundle, load];
    if !loaded {
        return false;
    }

    let center_cls = match Class::get("UNUserNotificationCenter") {
        Some(cls) => cls,
        None => return false,
    };
    let center: *mut Object = msg_send![center_cls, currentNotificationCenter];
    if center.is_null() {
        return false;
    }

    setup_notification_delegate_and_auth(center);
    deliver_notification(center, title_ns, sub_ns, body_ns)
}

/// One-time delegate registration and authorization request.
/// Registers a `UNUserNotificationCenterDelegate` so notifications appear even
/// when the app is in the foreground, and requests alert+sound authorization.
#[cfg(target_os = "macos")]
unsafe fn setup_notification_delegate_and_auth(center: *mut objc::runtime::Object) {
    use objc::declare::ClassDecl;
    use objc::runtime::{Class, Object, Sel};
    use objc::{msg_send, sel, sel_impl};
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
        fn _Block_copy(block: *const std::ffi::c_void) -> *mut std::ffi::c_void;
        fn _Block_release(block: *const std::ffi::c_void);
    }

    unsafe extern "C" fn noop_auth(_b: *mut AuthBlock, _granted: u8, _err: *mut Object) {}

    static AUTH_DESC: BlockDesc = BlockDesc {
        reserved: 0,
        size: std::mem::size_of::<AuthBlock>(),
    };

    /// Block layout for `willPresentNotification:withCompletionHandler:`.
    /// Apple signature: `void (^)(UNNotificationPresentationOptions)` where
    /// `UNNotificationPresentationOptions` is `NSUInteger` (`usize`).
    #[repr(C)]
    struct PresentCompletionBlock {
        _isa: usize,
        _flags: i32,
        _reserved: i32,
        invoke: unsafe extern "C" fn(*const std::ffi::c_void, usize),
    }

    /// Block layout for `didReceiveNotificationResponse:withCompletionHandler:`.
    /// Apple signature: `void (^)(void)` — no arguments beyond the block pointer.
    #[repr(C)]
    struct ResponseCompletionBlock {
        _isa: usize,
        _flags: i32,
        _reserved: i32,
        invoke: unsafe extern "C" fn(*const std::ffi::c_void),
    }

    static SETUP_ONCE: Once = Once::new();

    // We need to pass `center` into the Once closure, but the closure is
    // `FnOnce` and captures by value. The raw pointer is `Send`-safe for our
    // single-threaded UI usage.
    let center_ptr = center;

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
                let bh = handler as *const PresentCompletionBlock;
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
                let bh = handler as *const ResponseCompletionBlock;
                ((*bh).invoke)(handler);
            }
        }

        let superclass = Class::get("NSObject").unwrap();
        if let Some(mut decl) = ClassDecl::new("DirigentNotifDelegate", superclass) {
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
            let _: () = msg_send![center_ptr, setDelegate: delegate];
            // Intentionally leaked — the delegate must outlive the app.
        }

        // ── Authorization ──
        let auth_block = AuthBlock {
            isa: &_NSConcreteStackBlock as *const std::ffi::c_void,
            flags: 0,
            reserved: 0,
            invoke: noop_auth,
            descriptor: &AUTH_DESC,
        };
        // Copy the stack block to the heap so the async API can safely
        // invoke it after this stack frame returns.
        let auth_heap = _Block_copy(&auth_block as *const _ as *const std::ffi::c_void);
        // UNAuthorizationOptionAlert (1<<2) | Sound (1<<1)
        let opts: usize = 4 | 2;
        let _: () = msg_send![center_ptr,
            requestAuthorizationWithOptions:opts
            completionHandler:auth_heap];
        _Block_release(auth_heap);
    });
}

/// Build `UNMutableNotificationContent`, create a `UNNotificationRequest`,
/// and deliver it via the given `UNUserNotificationCenter`. Returns `true`
/// on success.
#[cfg(target_os = "macos")]
unsafe fn deliver_notification(
    center: *mut objc::runtime::Object,
    title_ns: *mut objc::runtime::Object,
    sub_ns: *mut objc::runtime::Object,
    body_ns: *mut objc::runtime::Object,
) -> bool {
    use objc::runtime::{Class, Object};
    use objc::{msg_send, sel, sel_impl};

    let content_cls = match Class::get("UNMutableNotificationContent") {
        Some(cls) => cls,
        None => return false,
    };
    let content: *mut Object = msg_send![content_cls, new];
    // Autorelease so the caller's NSAutoreleasePool handles cleanup,
    // covering both early-return and normal exit paths.
    let content: *mut Object = msg_send![content, autorelease];
    let _: () = msg_send![content, setTitle: title_ns];
    let _: () = msg_send![content, setSubtitle: sub_ns];
    let _: () = msg_send![content, setBody: body_ns];

    let request_cls = match Class::get("UNNotificationRequest") {
        Some(cls) => cls,
        None => return false,
    };
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
    true
}

/// Fallback notification via `osascript display notification`.
/// Used when running without a bundle identifier (e.g. `cargo run`).
#[cfg(target_os = "macos")]
fn fallback_osascript_notification(title: &str, subtitle: &str, body: &str) {
    let title_esc = title.replace('\\', "\\\\").replace('"', "\\\"");
    let sub_esc = subtitle.replace('\\', "\\\\").replace('"', "\\\"");
    let body_esc = body.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        "display notification \"{}\" with title \"{}\" subtitle \"{}\"",
        body_esc, title_esc, sub_esc
    );
    match std::process::Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(&script)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => {
            eprintln!("fallback notification failed to spawn osascript: {e}");
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn send_macos_notification(_title: &str, _subtitle: &str, _body: &str) {}
