//! Linux notifications over the freedesktop D-Bus service (ADR 0035).
//!
//! Called before the `notify-send` fallback, so a desktop with a session bus
//! gets a notification that belongs to mtty instead of to the `notify-send`
//! helper process.

use std::collections::HashMap;

use zbus::zvariant::Value;

/// Post through `org.freedesktop.Notifications`. Returns false when there is no
/// session bus or no notification service, so the caller falls back to
/// `notify-send`.
pub fn notify(title: &str, body: &str) -> bool {
    let connection = match zbus::blocking::Connection::session() {
        Ok(connection) => connection,
        Err(_) => return false,
    };
    let hints: HashMap<&str, Value<'_>> = HashMap::new();
    connection
        .call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &("mtty", 0u32, "", title, body, &[] as &[&str], &hints, -1i32),
        )
        .is_ok()
}
