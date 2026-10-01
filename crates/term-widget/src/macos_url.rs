//! macOS hands `mtty://` (and `ssh://`, `x-man-page://`) links opened from a
//! browser or Finder to the app as an Apple Event (`kAEGetURL`), not as argv,
//! and winit has no API for it. This registers a handler with
//! `NSAppleEventManager`, queues the URLs and wakes the event loop, which
//! applies them like command-line launches (ADR 0019).
//!
//! Plain `msg_send!` keeps the dependency to objc2 and NSString; the typed
//! bindings for these calls would pull in CoreServices.

use std::sync::{Mutex, OnceLock};

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, NSObject};
use objc2::{define_class, msg_send, sel, AllocAnyThread};
use objc2_foundation::NSString;

/// `kInternetEventClass` / `kAEGetURL` ('GURL') and `keyDirectObject` ('----').
const GURL: u32 = u32::from_be_bytes(*b"GURL");
const KEY_DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

static PENDING: Mutex<Vec<String>> = Mutex::new(Vec::new());
static WAKE: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

define_class!(
    // SAFETY: NSObject has no subclassing requirements and the class has no
    // Drop implementation.
    #[unsafe(super = NSObject)]
    #[name = "MttyUrlEventHandler"]
    struct UrlEventHandler;

    impl UrlEventHandler {
        // SAFETY: the signature matches the selector registered below.
        #[unsafe(method(handleGetURLEvent:withReplyEvent:))]
        fn handle_get_url(&self, event: &AnyObject, _reply: &AnyObject) {
            // SAFETY: `event` is an NSAppleEventDescriptor; both methods exist
            // on it and return autoreleased objects (or nil).
            let url = unsafe {
                let desc: Option<Retained<AnyObject>> =
                    msg_send![event, paramDescriptorForKeyword: KEY_DIRECT_OBJECT];
                desc.and_then(|d| {
                    let s: Option<Retained<NSString>> = msg_send![&*d, stringValue];
                    s
                })
            };
            if let Some(url) = url {
                PENDING
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(url.to_string());
                if let Some(wake) = WAKE.get() {
                    wake();
                }
            }
        }
    }
);

/// Register the handler. Call before the event loop runs, so the URL that
/// launched the app is caught too.
pub fn install(wake: impl Fn() + Send + Sync + 'static) {
    let _ = WAKE.set(Box::new(wake));
    let Some(class) = AnyClass::get(c"NSAppleEventManager") else {
        return;
    };
    // SAFETY: standard Foundation calls with matching argument types
    // (AEEventClass and AEEventID are 32-bit four-char codes).
    unsafe {
        let manager: Option<Retained<AnyObject>> = msg_send![class, sharedAppleEventManager];
        let Some(manager) = manager else {
            return;
        };
        let handler: Retained<UrlEventHandler> = msg_send![UrlEventHandler::alloc(), init];
        let _: () = msg_send![
            &*manager,
            setEventHandler: &*handler,
            andSelector: sel!(handleGetURLEvent:withReplyEvent:),
            forEventClass: GURL,
            andEventID: GURL
        ];
        // The event manager does not retain its handler.
        std::mem::forget(handler);
    }
}

/// URLs received since the last call.
pub fn take() -> Vec<String> {
    std::mem::take(&mut *PENDING.lock().unwrap_or_else(|e| e.into_inner()))
}
