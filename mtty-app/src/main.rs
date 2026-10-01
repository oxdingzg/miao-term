//! The mtty application: the native winit/wgpu host.

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match std::env::args().nth(1).as_deref() {
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
