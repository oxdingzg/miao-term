//! The miaotty application: the native winit/wgpu host.

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match std::env::args().nth(1).as_deref() {
        Some("--version" | "-V") => {
            println!("miaotty {} (native)", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("--help" | "-h") => {
            println!("miaotty — native terminal\nusage: miaotty [--quick | --focus PANE | URL]\nURLs: miaotty://, ssh://, x-man-page://");
            Ok(())
        }
        _ => miao_term_widget::run("miaotty"),
    }
}
