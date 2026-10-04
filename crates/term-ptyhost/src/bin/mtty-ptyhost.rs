//! `mtty-ptyhost`: keeps one pane's program running across mtty restarts
//! (ADR 0041). Started by mtty, never by hand.

fn main() {
    let args = match mtty_ptyhost::host::HostArgs::parse(std::env::args_os().skip(1)) {
        Ok(args) => args,
        Err(e) => {
            eprintln!("mtty-ptyhost: {e}");
            std::process::exit(2);
        }
    };
    match mtty_ptyhost::host::run(args) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("mtty-ptyhost: {e}");
            std::process::exit(1);
        }
    }
}
