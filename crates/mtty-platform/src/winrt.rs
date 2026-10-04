//! Windows notifications through WinRT (ADR 0035).
//!
//! The BurntToast PowerShell fallback needs a module that may be absent and
//! attributes the toast to PowerShell. Posting through `Windows.UI.Notifications`
//! under mtty's AppUserModelID makes the toast belong to mtty and needs no
//! external tool. When WinRT is unavailable the caller keeps BurntToast.

use windows::core::HSTRING;
use windows::Data::Xml::Dom::XmlDocument;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};

/// The application identity used by the installers (APP-IDENTITY).
const AUMID: &str = "dev.mtty.terminal";

/// Post a toast under mtty's AUMID. Returns false when WinRT is unavailable, so
/// the caller falls back to BurntToast.
pub fn notify(title: &str, body: &str) -> bool {
    // SAFETY: both calls are best-effort initialization; failures are ignored
    // and a desktop process has usually already initialized WinRT.
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
        let _ = SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(AUMID));
    }
    show(title, body).is_ok()
}

fn show(title: &str, body: &str) -> windows::core::Result<()> {
    let xml = XmlDocument::new()?;
    xml.LoadXml(&HSTRING::from(format!(
        "<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual></toast>",
        escape_xml(title),
        escape_xml(body)
    )))?;
    let toast = ToastNotification::CreateToastNotification(&xml)?;
    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))?;
    notifier.Show(&toast)
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
