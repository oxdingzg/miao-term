//! macOS hands `mtty://` (and `ssh://`, `x-man-page://`) links opened from a
//! browser or Finder to the app as an Apple Event (`kAEGetURL`), not as argv,
//! and winit has no API for it. This registers a handler with
//! `NSAppleEventManager`, queues the URLs and wakes the event loop, which
//! applies them like command-line launches (ADR 0019).
//!
//! The same handler takes the quit Apple Event (`kAEQuitApplication`) that
//! the Dock, logout, shutdown and `osascript … to quit` send: AppKit's own
//! handler terminates the process at once, skipping mtty's quit (which
//! saves the session and the terminals' contents), so it is routed there.
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

/// `kCoreEventClass` / `kAEQuitApplication`.
const AEVT: u32 = u32::from_be_bytes(*b"aevt");
const QUIT: u32 = u32::from_be_bytes(*b"quit");

static PENDING: Mutex<Vec<String>> = Mutex::new(Vec::new());
static QUIT_ASKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
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

        // SAFETY: the signature matches the selector registered below.
        #[unsafe(method(handleQuitEvent:withReplyEvent:))]
        fn handle_quit(&self, _event: &AnyObject, _reply: &AnyObject) {
            QUIT_ASKED.store(true, std::sync::atomic::Ordering::SeqCst);
            if let Some(wake) = WAKE.get() {
                wake();
            }
        }
    }
);

fn register(selector: objc2::runtime::Sel, class_id: u32, event_id: u32) {
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
            andSelector: selector,
            forEventClass: class_id,
            andEventID: event_id
        ];
        // The event manager does not retain its handler.
        std::mem::forget(handler);
    }
}

/// Register the handler. Call before the event loop runs, so the URL that
/// launched the app is caught too.
pub fn install(wake: impl Fn() + Send + Sync + 'static) {
    let _ = WAKE.set(Box::new(wake));
    register(sel!(handleGetURLEvent:withReplyEvent:), GURL, GURL);
}

/// Take over the quit Apple Event. Call once the app has finished launching:
/// AppKit installs its own quit handler during launch, replacing an earlier
/// one.
pub fn install_quit() {
    register(sel!(handleQuitEvent:withReplyEvent:), AEVT, QUIT);
}

/// Whether the OS asked the app to quit since the last call.
pub fn take_quit() -> bool {
    QUIT_ASKED.swap(false, std::sync::atomic::Ordering::SeqCst)
}

/// `CGPoint` / `CGSize` / `CGRect`, declared here so the calls below need no
/// CoreGraphics bindings.
#[repr(C)]
#[derive(Clone, Copy)]
struct Point {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Rect {
    origin: Point,
    size: Point,
}

// SAFETY: the layouts match CGPoint, CGSize and CGRect on 64-bit macOS.
unsafe impl objc2::Encode for Point {
    const ENCODING: objc2::Encoding =
        objc2::Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
}
unsafe impl objc2::Encode for Rect {
    const ENCODING: objc2::Encoding = objc2::Encoding::Struct(
        "CGRect",
        &[
            Point::ENCODING,
            objc2::Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]),
        ],
    );
}

/// The pointer in points from the main screen's top-left corner, the
/// coordinates winit's window positions use (before scaling). A file drag
/// sends the window no pointer motion, so this is where a drop landed.
pub fn pointer_on_screen() -> Option<(f64, f64)> {
    let event = AnyClass::get(c"NSEvent")?;
    let screen = AnyClass::get(c"NSScreen")?;
    // SAFETY: class methods with these signatures; `screens` is never empty
    // on a running desktop and `firstObject` is nil-safe.
    unsafe {
        let at: Point = msg_send![event, mouseLocation];
        let screens: Option<Retained<AnyObject>> = msg_send![screen, screens];
        let main: Option<Retained<AnyObject>> = msg_send![&*screens?, firstObject];
        let frame: Rect = msg_send![&*main?, frame];
        Some((at.x, frame.size.y - at.y))
    }
}

/// URLs received since the last call.
pub fn take() -> Vec<String> {
    std::mem::take(&mut *PENDING.lock().unwrap_or_else(|e| e.into_inner()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_pointer_is_read_on_the_main_screen() {
        // objc2 checks the declared struct encodings against the runtime's
        // in debug builds, so a wrong layout fails here, not on a drop.
        let Some((x, y)) = super::pointer_on_screen() else {
            return; // no window server (a headless runner)
        };
        assert!(x.is_finite() && y.is_finite());
        assert!(y > -10_000.0 && y < 10_000.0, "{y}");
    }
}
