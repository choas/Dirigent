/// Outcome of attempting to send a notification via the modern
/// `UNUserNotificationCenter` API.
#[cfg(target_os = "macos")]
enum NotificationOutcome {
    /// Notification was successfully scheduled for delivery.
    Delivered,
    /// The modern notification API is not available (no bundle identifier,
    /// framework not present, or required classes missing). The caller should
    /// fall back to `osascript`.
    Unavailable,
    /// The user has not authorized notifications for this app.
    NotAuthorized,
    /// Delivery failed for another reason (timeout, scheduling error, etc.).
    Failed(String),
}

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

        match try_modern_notification(title_ns, sub_ns, body_ns) {
            NotificationOutcome::Unavailable => {
                // Fallback: osascript `display notification`.
                // Used when running without a bundle identifier (e.g. `cargo run`).
                fallback_osascript_notification(&title_safe, &subtitle_safe, &body_safe);
            }
            NotificationOutcome::Delivered => {}
            NotificationOutcome::NotAuthorized => {
                eprintln!("macOS notifications not authorized for this app");
            }
            NotificationOutcome::Failed(e) => {
                eprintln!("notification delivery failed: {e}");
            }
        }

        let _: () = msg_send![pool, drain];
    }
}

/// Try to send a notification via the modern `UNUserNotificationCenter` API
/// (macOS 10.14+). Returns a [`NotificationOutcome`] indicating whether the
/// notification was delivered, the API is unavailable, the user has not
/// authorized notifications, or delivery failed.
#[cfg(target_os = "macos")]
unsafe fn try_modern_notification(
    title_ns: *mut objc::runtime::Object,
    sub_ns: *mut objc::runtime::Object,
    body_ns: *mut objc::runtime::Object,
) -> NotificationOutcome {
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
        return NotificationOutcome::Unavailable;
    }

    let loaded: bool = msg_send![fw_bundle, load];
    if !loaded {
        return NotificationOutcome::Unavailable;
    }

    let center_cls = match Class::get("UNUserNotificationCenter") {
        Some(cls) => cls,
        None => return NotificationOutcome::Unavailable,
    };
    let center: *mut Object = msg_send![center_cls, currentNotificationCenter];
    if center.is_null() {
        return NotificationOutcome::Unavailable;
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
/// and deliver it via the given `UNUserNotificationCenter`.
/// Returns a [`NotificationOutcome`] based on the completion handler result
/// or timeout.
#[cfg(target_os = "macos")]
unsafe fn deliver_notification(
    center: *mut objc::runtime::Object,
    title_ns: *mut objc::runtime::Object,
    sub_ns: *mut objc::runtime::Object,
    body_ns: *mut objc::runtime::Object,
) -> NotificationOutcome {
    use objc::runtime::{Class, Object};
    use objc::{msg_send, sel, sel_impl};

    extern "C" {
        static _NSConcreteStackBlock: std::ffi::c_void;
        fn _Block_copy(block: *const std::ffi::c_void) -> *mut std::ffi::c_void;
        fn _Block_release(block: *const std::ffi::c_void);
        fn dispatch_semaphore_create(value: isize) -> *const std::ffi::c_void;
        fn dispatch_semaphore_signal(dsema: *const std::ffi::c_void) -> isize;
        fn dispatch_semaphore_wait(dsema: *const std::ffi::c_void, timeout: u64) -> isize;
        fn dispatch_time(when: u64, delta: i64) -> u64;
        fn dispatch_release(object: *const std::ffi::c_void);
    }

    #[repr(C)]
    struct CompletionBlockDesc {
        reserved: usize,
        size: usize,
    }

    /// Block layout for `void (^)(NSError *)` with captured state for
    /// synchronously reporting the scheduling result back to Rust.
    #[repr(C)]
    struct CompletionBlock {
        isa: *const std::ffi::c_void,
        flags: i32,
        reserved: i32,
        invoke: unsafe extern "C" fn(*mut CompletionBlock, *mut Object),
        descriptor: *const CompletionBlockDesc,
        outcome: *mut u8,
        semaphore: *const std::ffi::c_void,
    }

    static COMPLETION_DESC: CompletionBlockDesc = CompletionBlockDesc {
        reserved: 0,
        size: std::mem::size_of::<CompletionBlock>(),
    };

    // Outcome codes written by the completion handler:
    // 0 = delivered, 1 = failed, 2 = not authorized.
    const OUTCOME_DELIVERED: u8 = 0;
    const OUTCOME_FAILED: u8 = 1;
    const OUTCOME_NOT_AUTHORIZED: u8 = 2;

    unsafe extern "C" fn completion_invoke(block: *mut CompletionBlock, error: *mut Object) {
        use objc::{msg_send, sel, sel_impl};

        if error.is_null() {
            *(*block).outcome = OUTCOME_DELIVERED;
        } else {
            let code: isize = msg_send![error, code];
            // UNErrorCodeNotificationsNotAllowed == 1
            *(*block).outcome = if code == 1 {
                OUTCOME_NOT_AUTHORIZED
            } else {
                OUTCOME_FAILED
            };
        }
        dispatch_semaphore_signal((*block).semaphore);
    }

    let content_cls = match Class::get("UNMutableNotificationContent") {
        Some(cls) => cls,
        None => return NotificationOutcome::Unavailable,
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
        None => return NotificationOutcome::Unavailable,
    };
    let nsuuid_cls = Class::get("NSUUID").unwrap();
    let uuid: *mut Object = msg_send![nsuuid_cls, UUID];
    let uuid_str: *mut Object = msg_send![uuid, UUIDString];
    let trigger: *const Object = std::ptr::null();
    let request: *mut Object = msg_send![request_cls,
        requestWithIdentifier:uuid_str
        content:content
        trigger:trigger];

    // Heap-allocate `outcome` so that a late-firing completion handler
    // (after timeout) does not write to a dead stack slot.
    let outcome_ptr = Box::into_raw(Box::new(OUTCOME_FAILED));
    let semaphore = dispatch_semaphore_create(0);

    let stack_block = CompletionBlock {
        isa: &_NSConcreteStackBlock as *const std::ffi::c_void,
        flags: 0,
        reserved: 0,
        invoke: completion_invoke,
        descriptor: &COMPLETION_DESC,
        outcome: outcome_ptr,
        semaphore,
    };

    // Copy to heap so the async callback can safely invoke the block
    // after this stack frame proceeds to the semaphore wait.
    let heap_block = _Block_copy(&stack_block as *const _ as *const std::ffi::c_void);

    let _: () = msg_send![center,
        addNotificationRequest:request
        withCompletionHandler:heap_block];

    // Wait up to 2 seconds for the completion handler.
    let timeout = dispatch_time(0, 2_000_000_000);
    let wait_result = dispatch_semaphore_wait(semaphore, timeout);

    if wait_result != 0 {
        // Timed out — intentionally leak `outcome_ptr`, `heap_block`, and
        // `semaphore` so a late-firing completion handler can still safely
        // dereference the block's captured pointers and signal the semaphore.
        // The leak is tiny (one u8 + one ObjC block + one semaphore) and
        // only occurs when the notification system fails to respond in time.
        return NotificationOutcome::Failed("timed out waiting for notification completion".into());
    }

    // Completion handler ran within the timeout — safe to read and free.
    let outcome_code = *outcome_ptr;
    drop(Box::from_raw(outcome_ptr));
    _Block_release(heap_block);
    dispatch_release(semaphore);

    match outcome_code {
        OUTCOME_DELIVERED => NotificationOutcome::Delivered,
        OUTCOME_NOT_AUTHORIZED => NotificationOutcome::NotAuthorized,
        _ => NotificationOutcome::Failed("notification scheduling error".into()),
    }
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
