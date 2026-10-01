//! The mtty application: the native winit/wgpu host.
// A GUI program on Windows: without this every start (Start menu, shortcut,
// MSI) also opened a console window, and closing it killed mtty.
#![cfg_attr(windows, windows_subsystem = "windows")]

/// `--version` / `--help` from cmd or PowerShell: a GUI program has no console,
/// so print into the parent's (redirected output works without this).
fn attach_parent_console() {
    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::System::Console::AttachConsole(
            windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS,
        );
    }
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let arg = std::env::args().nth(1);
    if matches!(arg.as_deref(), Some("--version" | "-V" | "--help" | "-h")) {
        attach_parent_console();
    }
    match arg.as_deref() {
        Some("--version" | "-V") => {
            println!("mtty {} (native)", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("--help" | "-h") => {
            println!("mtty — native terminal\nusage: mtty [--quick | --focus PANE | URL]\nURLs: mtty:// (or miaotty://), ssh://, x-man-page://");
            Ok(())
        }
        _ => miao_term_widget::run("mtty"),
    }
}
