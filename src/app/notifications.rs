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

/// Create an `NSString` from a Rust `&str`, returning a raw pointer suitable
/// for passing to ObjC message sends.
///
/// # Safety
/// Must be called within an active `NSAutoreleasePool` scope. The returned
/// pointer is autoreleased and valid until the enclosing pool is drained.
/// The input must not contain interior null bytes (caller strips them).
#[cfg(target_os = "macos")]
unsafe fn ns_string_from_str(s: &str) -> *mut objc::runtime::Object {
    use objc::runtime::Class;
    use objc::{msg_send, sel, sel_impl};

    let nsstring = Class::get("NSString").unwrap();
    let c_str = std::ffi::CString::new(s).expect("interior null bytes already stripped");
    // SAFETY: `NSString stringWithUTF8String:` is safe to call with a valid
    // C string pointer. The returned object is autoreleased by the runtime.
    msg_send![nsstring, stringWithUTF8String: c_str.as_ptr()]
}

/// Send a macOS notification via `UNUserNotificationCenter` (modern API).
/// Registers a delegate on first call so notifications are shown even when the
/// app is in the foreground (macOS suppresses them by default).
/// Falls back to `osascript display notification` when running without a bundle.
#[cfg(target_os = "macos")]
pub(super) fn send_macos_notification(title: &str, subtitle: &str, body: &str) {
    use objc::runtime::{Class, Object};
    use objc::{msg_send, sel, sel_impl};

    // Strip null bytes to prevent CString::new panics.
    let title_safe = title.replace('\0', "");
    let subtitle_safe = subtitle.replace('\0', "");
    let body_safe = body.replace('\0', "");

    // SAFETY: All ObjC messaging in this block operates within an
    // NSAutoreleasePool that is drained before the block exits. The
    // NSString pointers (`title_ns`, `sub_ns`, `body_ns`) are autoreleased
    // and remain valid for the duration of the pool. Class lookups
    // (`NSAutoreleasePool`, `NSString`) are guaranteed to exist on any
    // supported macOS version (10.14+). Interior null bytes have been
    // stripped above, so CString construction cannot panic.
    unsafe {
        // SAFETY: `NSAutoreleasePool` is always available on macOS.
        let pool_cls = Class::get("NSAutoreleasePool").unwrap();
        let pool: *mut Object = msg_send![pool_cls, new];

        let title_ns = ns_string_from_str(&title_safe);
        let sub_ns = ns_string_from_str(&subtitle_safe);
        let body_ns = ns_string_from_str(&body_safe);

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

        // SAFETY: `drain` releases all autoreleased objects created since
        // `[pool new]`. After this point, `title_ns`/`sub_ns`/`body_ns` are
        // invalid — but they are not used again.
        let _: () = msg_send![pool, drain];
    }
}

/// Try to send a notification via the modern `UNUserNotificationCenter` API
/// (macOS 10.14+). Returns a [`NotificationOutcome`] indicating whether the
/// notification was delivered, the API is unavailable, the user has not
/// authorized notifications, or delivery failed.
///
/// # Safety
/// - `title_ns`, `sub_ns`, and `body_ns` must be valid, autoreleased
///   `NSString*` pointers (or ObjC `nil`). ObjC nil-messaging is safe (returns
///   zero/nil), so null pointers degrade gracefully — notifications just appear
///   with empty fields rather than crashing.
/// - Must be called within an active `NSAutoreleasePool` scope so that
///   autoreleased intermediaries (framework path string, UUID, etc.) are
///   collected.
#[cfg(target_os = "macos")]
unsafe fn try_modern_notification(
    title_ns: *mut objc::runtime::Object,
    sub_ns: *mut objc::runtime::Object,
    body_ns: *mut objc::runtime::Object,
) -> NotificationOutcome {
    use objc::runtime::{Class, Object};
    use objc::{msg_send, sel, sel_impl};

    // SAFETY: `NSBundle` is always present on macOS.
    let bundle_cls = Class::get("NSBundle").unwrap();

    // SAFETY: The path is a compile-time constant with no interior nulls.
    let fw_path_ns = ns_string_from_str("/System/Library/Frameworks/UserNotifications.framework");
    // SAFETY: `bundleWithPath:` returns nil for non-existent paths — checked below.
    let fw_bundle: *mut Object = msg_send![bundle_cls, bundleWithPath: fw_path_ns];

    // SAFETY: `mainBundle` always returns a valid object; `bundleIdentifier`
    // returns nil when running without a bundle (e.g. `cargo run`).
    let main_bundle: *mut Object = msg_send![bundle_cls, mainBundle];
    let bundle_id: *mut Object = msg_send![main_bundle, bundleIdentifier];

    if fw_bundle.is_null() || bundle_id.is_null() {
        return NotificationOutcome::Unavailable;
    }

    // SAFETY: `fw_bundle` is non-null (checked above). `load` is idempotent
    // and returns false only if the framework binary is missing/corrupt.
    let loaded: bool = msg_send![fw_bundle, load];
    if !loaded {
        return NotificationOutcome::Unavailable;
    }

    // SAFETY: After successfully loading the framework, the class should be
    // registered with the ObjC runtime. If it is not, we treat the API as
    // unavailable rather than panicking.
    let center_cls = match Class::get("UNUserNotificationCenter") {
        Some(cls) => cls,
        None => return NotificationOutcome::Unavailable,
    };
    // SAFETY: `currentNotificationCenter` returns nil if the notification
    // system cannot be initialized — checked below.
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
///
/// # Safety
/// - `center` must be a valid, non-null `UNUserNotificationCenter*` pointer.
/// - Must be called from the main thread (ObjC delegate assignment and
///   authorization requests are main-thread-only operations).
/// - The delegate object is intentionally leaked so it outlives the app;
///   this is safe because macOS reclaims all process memory on exit.
/// - The `AuthBlock` is stack-allocated, then heap-copied via `_Block_copy`
///   before being passed to the async authorization API. `_Block_release` is
///   called immediately after — the runtime retains its own copy.
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
            // SAFETY: `handler` is a valid ObjC block pointer provided by
            // the system. We cast it to `PresentCompletionBlock` which
            // matches the ABI layout of `void (^)(UNNotificationPresentationOptions)`.
            // The block is invoked exactly once, as required by the API contract.
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
            // SAFETY: `handler` is a valid ObjC block pointer provided by
            // the system. We cast it to `ResponseCompletionBlock` which
            // matches the ABI layout of `void (^)(void)`. The block is
            // invoked exactly once, as required by the API contract.
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
///
/// # Safety
/// - `center` must be a valid, non-null `UNUserNotificationCenter*` pointer.
/// - `title_ns`, `sub_ns`, and `body_ns` must be valid `NSString*` pointers
///   (or ObjC nil). Nil is safe — ObjC nil-messaging is a no-op.
/// - Must be called within an active `NSAutoreleasePool` scope.
/// - The `CompletionBlock` is stack-allocated and heap-copied via `_Block_copy`
///   before being passed to `addNotificationRequest:withCompletionHandler:`.
///   The heap block captures a `Box`-allocated `outcome_ptr` and a GCD
///   semaphore; on timeout, both are intentionally leaked so the late-firing
///   completion handler does not write to freed memory.
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

    // SAFETY: Called by the ObjC runtime as a block invocation. `block` is a
    // valid pointer to the heap-copied `CompletionBlock`. `outcome` is a
    // heap-allocated `Box<u8>` (via `Box::into_raw`) that remains valid because:
    //   - On the happy path, we wait on the semaphore before freeing it.
    //   - On timeout, we intentionally leak it so this write is still safe.
    // `error` is either a valid `NSError*` or ObjC nil (checked before use).
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

    // SAFETY: Class lookups use `match` — None returns Unavailable, not UB.
    // `UNMutableNotificationContent` was loaded by `try_modern_notification`
    // before calling us.
    let content_cls = match Class::get("UNMutableNotificationContent") {
        Some(cls) => cls,
        None => return NotificationOutcome::Unavailable,
    };
    let content: *mut Object = msg_send![content_cls, new];
    // SAFETY: Autorelease so the caller's NSAutoreleasePool handles cleanup,
    // covering both early-return and normal exit paths.
    let content: *mut Object = msg_send![content, autorelease];
    // SAFETY: Setters accept NSString* (or nil). ObjC nil-messaging is a no-op.
    let _: () = msg_send![content, setTitle: title_ns];
    let _: () = msg_send![content, setSubtitle: sub_ns];
    let _: () = msg_send![content, setBody: body_ns];

    let request_cls = match Class::get("UNNotificationRequest") {
        Some(cls) => cls,
        None => return NotificationOutcome::Unavailable,
    };
    // SAFETY: `NSUUID` is always available on macOS 10.8+. `UUID` and
    // `UUIDString` return autoreleased objects valid within the pool scope.
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

    // SAFETY: Completion handler ran within the timeout — the semaphore
    // guarantees `outcome_ptr` has been written and no concurrent access
    // remains. Safe to read the value and reclaim the Box.
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
