//! Native macOS notifications through `UserNotifications` (ADR 0035).
//!
//! `UNUserNotificationCenter` is a per-app center keyed by the bundle
//! identifier: notifications it posts belong to mtty (icon, name, click), not
//! to Script Editor the way `/usr/bin/osascript` does. It also has no bundle
//! proxy outside an app bundle and raises, so [`notify`] reports failure and
//! the caller keeps the AppleScript fallback.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use objc2::rc::Retained;
use objc2::runtime::{Bool, NSObject, ProtocolObject};
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
