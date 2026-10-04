//! Native macOS notifications through `UserNotifications` (ADR 0035).
//!
//! `UNUserNotificationCenter` is a per-app center keyed by the bundle
//! identifier: notifications it posts belong to mtty (icon, name, click), not
//! to Script Editor the way `/usr/bin/osascript` does. It also has no bundle
//! proxy outside an app bundle and raises, so [`notify`] reports failure and
//! the caller keeps the AppleScript fallback.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, NSObject, ProtocolObject};
use objc2::{define_class, msg_send, AllocAnyThread};
use objc2_foundation::{NSBundle, NSError, NSObjectProtocol, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification,
    UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationSound,
    UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};

/// The delegate is installed at most once. Its center property is weak, so the
/// object is leaked on purpose below.
static DELEGATE_INSTALLED: AtomicBool = AtomicBool::new(false);

/// Notification identifiers only need to be unique per request; reusing one
/// replaces the previous banner, which would hide distinct agent events.
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

define_class!(
    // SAFETY: NSObject has no subclassing requirements and the class has no
    // Drop implementation.
    #[unsafe(super = NSObject)]
    #[name = "MttyNotificationDelegate"]
    struct NotificationDelegate;

    unsafe impl NSObjectProtocol for NotificationDelegate {}

    unsafe impl UNUserNotificationCenterDelegate for NotificationDelegate {
        // Show the banner even while mtty is frontmost: the notification is
        // for a pane that is not the focused one (ADR 0010).
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            completion_handler: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            completion_handler.call((UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::Sound,));
        }
    }
);

/// Post a notification through the bundle's notification center. Returns false
/// when mtty is not bundled, so the caller falls back to `osascript`.
pub fn notify(title: &str, body: &str) -> bool {
    if NSBundle::mainBundle().bundleIdentifier().is_none() {
        return false;
    }
    let center = UNUserNotificationCenter::currentNotificationCenter();
    install_delegate(&center);

    // Ask every time: the system only prompts once, and the authorization may
    // have been granted or changed since the last call. The banner is posted
    // from the completion so a first-run prompt cannot swallow it.
    let title = title.to_owned();
    let body = body.to_owned();
    let completion = block2::RcBlock::new(move |_granted: Bool, _error: *mut NSError| {
        deliver(&title, &body);
    });
    center.requestAuthorizationWithOptions_completionHandler(
        UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
        &completion,
    );
    true
}

fn install_delegate(center: &UNUserNotificationCenter) {
    if DELEGATE_INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    // SAFETY: `alloc`/`init` are the documented constructor pair for a class
    // defined above; the center's `delegate` is weak, so the one retained
    // object is intentionally leaked for the process lifetime.
    let delegate: Retained<NotificationDelegate> =
        unsafe { msg_send![NotificationDelegate::alloc(), init] };
    center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    std::mem::forget(delegate);
}

fn deliver(title: &str, body: &str) {
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setBody(&NSString::from_str(body));
    content.setSound(Some(&UNNotificationSound::defaultSound()));
    let identifier =
        NSString::from_str(&format!("mtty-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed)));
    let request =
        UNNotificationRequest::requestWithIdentifier_content_trigger(&identifier, &content, None);
    center.addNotificationRequest_withCompletionHandler(&request, None);
}

// NSPasteboard lives in AppKit; linking it guarantees the class is registered
// before `AnyClass::get` asks for it, even when nothing else in the process has
// loaded AppKit yet.
#[link(name = "AppKit", kind = "framework")]
extern "C" {}

/// The general pasteboard's image as PNG bytes, when it holds one. Reads
/// `public.png` directly and only transcodes the `public.tiff` representation a
/// screenshot leaves behind (ADR 0036).
pub fn clipboard_image() -> Option<Vec<u8>> {
    // SAFETY: standard NSPasteboard/NSData calls. Returned objects are retained
    // and every pointer is checked before use.
    unsafe {
        let pasteboard_class = AnyClass::get(c"NSPasteboard")?;
        let pasteboard: Retained<AnyObject> = msg_send![pasteboard_class, generalPasteboard];
        if let Some(png) = pasteboard_data(&pasteboard, "public.png") {
            return data_bytes(&png);
        }
        let tiff = pasteboard_data(&pasteboard, "public.tiff")?;
        let rep_class = AnyClass::get(c"NSBitmapImageRep")?;
        let rep: Retained<AnyObject> = msg_send![rep_class, imageRepWithData: &*tiff];
        // NSBitmapImageFileTypePNG = 4. `properties` is documented nullable.
        let png: Option<Retained<AnyObject>> = msg_send![
            &*rep,
            representationUsingType: 4isize,
            properties: core::ptr::null::<AnyObject>()
        ];
        let png = png?;
        data_bytes(&png)
    }
}

unsafe fn pasteboard_data(pasteboard: &AnyObject, kind: &str) -> Option<Retained<AnyObject>> {
    let kind = NSString::from_str(kind);
    // SAFETY: `dataForType:` takes an NSString and returns an autoreleased NSData.
    unsafe { msg_send![pasteboard, dataForType: &*kind] }
}

unsafe fn data_bytes(data: &AnyObject) -> Option<Vec<u8>> {
    // SAFETY: `bytes`/`length` are the documented NSData accessors; the buffer
    // is valid for `length` bytes for the lifetime of `data`.
    unsafe {
        let length: usize = msg_send![data, length];
        if length == 0 {
            return None;
        }
        // `bytes` returns `const void *`; the concrete pointer type must match
        // the encoding or objc2's debug signature check rejects the send.
        let bytes: *const core::ffi::c_void = msg_send![data, bytes];
        if bytes.is_null() {
            return None;
        }
        Some(core::slice::from_raw_parts(bytes.cast::<u8>(), length).to_vec())
    }
}
