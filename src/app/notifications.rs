/// Send a macOS notification via `UNUserNotificationCenter` (modern API).
/// Clicking the notification activates the Dirigent process that sent it.
/// Falls back to the deprecated `NSUserNotificationCenter`, then to `osascript`.
#[cfg(target_os = "macos")]
pub(super) fn send_macos_notification(title: &str, subtitle: &str, body: &str) {
    use objc::runtime::{Class, Object};
    use objc::{msg_send, sel, sel_impl};
    use std::ffi::CString;
    use std::process::Command;

    // Objective-C block layout (no captures) for completion handlers.
    #[repr(C)]
    struct ObjcBlock {
        isa: *const std::ffi::c_void,
        flags: i32,
        reserved: i32,
        invoke: unsafe extern "C" fn(*mut ObjcBlock, u8, *mut Object),
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

    unsafe extern "C" fn noop_auth(_block: *mut ObjcBlock, _granted: u8, _error: *mut Object) {}

    static BLOCK_DESC: BlockDesc = BlockDesc {
        reserved: 0,
        size: std::mem::size_of::<ObjcBlock>(),
    };

    unsafe {
        let pool_cls = Class::get("NSAutoreleasePool").unwrap();
        let pool: *mut Object = msg_send![pool_cls, new];

        let nsstring = Class::get("NSString").unwrap();

        // Strip null bytes to prevent CString::new panics
        let title_safe = title.replace('\0', "");
        let subtitle_safe = subtitle.replace('\0', "");
        let body_safe = body.replace('\0', "");

        let title_c = CString::new(title_safe).unwrap();
        let sub_c = CString::new(subtitle_safe).unwrap();
        let body_c = CString::new(body_safe).unwrap();

        let title_ns: *mut Object = msg_send![nsstring, stringWithUTF8String: title_c.as_ptr()];
        let sub_ns: *mut Object = msg_send![nsstring, stringWithUTF8String: sub_c.as_ptr()];
        let body_ns: *mut Object = msg_send![nsstring, stringWithUTF8String: body_c.as_ptr()];

        let mut delivered = false;

        // ── Modern API: UNUserNotificationCenter (macOS 10.14+) ──
        // Load the UserNotifications framework at runtime.
        let bundle_cls = Class::get("NSBundle").unwrap();
        let fw_path_c =
            CString::new("/System/Library/Frameworks/UserNotifications.framework").unwrap();
        let fw_path_ns: *mut Object = msg_send![nsstring, stringWithUTF8String: fw_path_c.as_ptr()];
        let fw_bundle: *mut Object = msg_send![bundle_cls, bundleWithPath: fw_path_ns];

        // UNUserNotificationCenter requires a bundle identifier; without one
        // (e.g. running from terminal) it throws an unrecoverable NSException.
        let main_bundle: *mut Object = msg_send![bundle_cls, mainBundle];
        let bundle_id: *mut Object = msg_send![main_bundle, bundleIdentifier];

        if !fw_bundle.is_null() && !bundle_id.is_null() {
            let loaded: bool = msg_send![fw_bundle, load];
            if loaded {
                if let Some(center_cls) = Class::get("UNUserNotificationCenter") {
                    let center: *mut Object = msg_send![center_cls, currentNotificationCenter];
                    if !center.is_null() {
                        // Request authorization (idempotent once granted).
                        let auth_block = ObjcBlock {
                            isa: &_NSConcreteStackBlock as *const _ as *const std::ffi::c_void,
                            flags: 0,
                            reserved: 0,
                            invoke: noop_auth,
                            descriptor: &BLOCK_DESC,
                        };
                        // UNAuthorizationOptionAlert (1<<2) | UNAuthorizationOptionSound (1<<1)
                        let options: usize = 4 | 2;
                        let _: () = msg_send![center,
                            requestAuthorizationWithOptions:options
                            completionHandler:&auth_block as *const _ as *const std::ffi::c_void];

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

        // ── Legacy fallback: NSUserNotificationCenter (pre-removal) ──
        if !delivered {
            if let (Some(notif_cls), Some(center_cls)) = (
                Class::get("NSUserNotification"),
                Class::get("NSUserNotificationCenter"),
            ) {
                let center: *mut Object = msg_send![center_cls, defaultUserNotificationCenter];
                if !center.is_null() {
                    let notif: *mut Object = msg_send![notif_cls, alloc];
                    let notif: *mut Object = msg_send![notif, init];

                    let _: () = msg_send![notif, setTitle: title_ns];
                    let _: () = msg_send![notif, setSubtitle: sub_ns];
                    let _: () = msg_send![notif, setInformativeText: body_ns];

                    let _: () = msg_send![center, deliverNotification: notif];
                    delivered = true;
                }
            }
        }

        // ── Final fallback: osascript (notification attributed to Script Editor) ──
        if !delivered {
            fn escape(s: &str) -> String {
                s.replace('\\', "\\\\").replace('"', "\\\"")
            }
            let script = format!(
                "display notification \"{}\" with title \"{}\" subtitle \"{}\"",
                escape(body),
                escape(title),
                escape(subtitle),
            );
            let _ = Command::new("osascript").arg("-e").arg(&script).output();
        }

        let _: () = msg_send![pool, drain];
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn send_macos_notification(_title: &str, _subtitle: &str, _body: &str) {}
