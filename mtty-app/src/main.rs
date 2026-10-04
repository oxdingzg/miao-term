//! The mtty application: the native winit/wgpu host.
// A GUI program on Windows: without this every start (Start menu, shortcut,
// MSI) also opened a console window, and closing it killed mtty.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod panic_log;
mod resource_monitor;

/// `--version` / `--help` from cmd or PowerShell: a GUI program has no console,
/// so print into the parent's (redirected output works without this).
fn attach_parent_console() {
    // Only when output goes nowhere: a redirect (`> file`, a pipe) already
    // gave the process a valid handle, and attaching would take it over.
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
        use windows_sys::Win32::System::Console::{
            AttachConsole, GetStdHandle, ATTACH_PARENT_PROCESS, STD_OUTPUT_HANDLE,
        };
        let out = GetStdHandle(STD_OUTPUT_HANDLE);
        if out.is_null() || out == INVALID_HANDLE_VALUE {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // First, so a panic on any thread leaves a record even without stderr.
    panic_log::install();
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
        _ => {
            resource_monitor::start();
            mtty_widget::run("mtty")
        }
    }
}
