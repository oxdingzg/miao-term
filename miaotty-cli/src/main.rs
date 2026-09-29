//! `miaotty-cli` — control CLI (cross-platform MTP client).
//!
//! Talks to the running miaotty host over the MTP socket. Examples:
//!   miaotty-cli ping
//!   miaotty-cli pane list
//!   miaotty-cli state miao --state processing [--pane ID]
//!   miaotty-cli state list
//!   miaotty-cli history add --command "ls" [--cwd DIR] [--pane ID]
//!   miaotty-cli history list [--pane ID]
//!   miaotty-cli pane run --pane ID --data "echo hi"
//!   miaotty-cli pane focus|close --pane ID

use std::path::PathBuf;

use serde_json::{json, Value};

fn usage() -> ! {
    eprintln!(
        "usage: miaotty-cli [--socket PATH] <command>\n\
         commands: ping | health | pane list|run|send|focus|close | \
         state <agent> --state S | state list | history add|list |\n     view|edit <path> |\n     file read --path P [--offset N] [--length N] [--base64] |\n     file write --path P [--data D | --data-b64 B]"
    );
    std::process::exit(2);
}

/// Read `--flag value` pairs starting at `args[start..]`.
fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();

    let mut socket = std::env::var("MIAOTTY_SOCKET")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(miao_term_mtp::default_socket);
    if let Some(i) = args.iter().position(|a| a == "--socket") {
        if i + 1 < args.len() {
            socket = PathBuf::from(args.remove(i + 1));
            args.remove(i);
        }
    }

    let pane_default = || std::env::var("MIAOTTY_PANE_ID").ok();
    let cmd = args.first().map(String::as_str).unwrap_or("");

    let mut client = match miao_term_mtp::client::connect(&socket) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("miaotty-cli: cannot connect to {}: {e}", socket.display());
            std::process::exit(1);
        }
    };

    let result: std::io::Result<Value> = match (cmd, args.get(1).map(String::as_str)) {
        ("ping", _) => client.call("core", "ping", json!({})),
        ("health", _) => client.call("core", "health", json!({})),
        ("pane", Some("list")) => client.call("pane", "list", json!({})),
        ("pane", Some(m @ ("run" | "send" | "focus" | "close"))) => {
            let pane = flag(&args, "--pane")
                .map(str::to_string)
                .or_else(pane_default);
            if m == "focus" || m == "close" {
                client.call("pane", m, json!({ "pane_id": pane }))
            } else {
                let data = flag(&args, "--data").unwrap_or("");
                client.call("pane", m, json!({ "pane_id": pane, "data": data }))
            }
        }
        ("view", Some(path)) | ("edit", Some(path)) => {
            client.call("app", cmd, json!({ "path": path }))
        }
        ("file", Some(m @ ("read" | "write"))) => {
            let path = flag(&args, "--path").unwrap_or("");
            if m == "read" {
                let mut params = json!({ "path": path });
                if let Some(offset) = flag(&args, "--offset").and_then(|v| v.parse::<u64>().ok()) {
                    params["offset"] = json!(offset);
                }
                if let Some(length) = flag(&args, "--length").and_then(|v| v.parse::<u64>().ok()) {
                    params["length"] = json!(length);
                }
                if args.iter().any(|a| a == "--base64") {
                    params["encoding"] = json!("base64");
                }
                client.call("file", "read", params)
            } else if let Some(b64) = flag(&args, "--data-b64") {
                client.call("file", "write", json!({ "path": path, "data_b64": b64 }))
            } else {
                let data = flag(&args, "--data").unwrap_or("");
                client.call("file", "write", json!({ "path": path, "data": data }))
            }
        }
        ("state", Some("list")) => client.call("agent", "state.list", json!({})),
        ("state", Some(agent)) => {
            let state = flag(&args, "--state").unwrap_or("");
            let pane = flag(&args, "--pane")
                .map(str::to_string)
                .or_else(pane_default);
            let session = flag(&args, "--session");
            client.call(
                "agent",
                "state.set",
                json!({ "agent": agent, "state": state, "pane_id": pane, "session_id": session }),
            )
        }
        ("history", Some("list")) => {
            let pane = flag(&args, "--pane")
                .map(str::to_string)
                .or_else(pane_default);
            client.call("history", "list", json!({ "pane_id": pane }))
        }
        ("history", Some("add")) => {
            let command = flag(&args, "--command").unwrap_or("");
            let cwd = flag(&args, "--cwd");
            let pane = flag(&args, "--pane")
                .map(str::to_string)
                .or_else(pane_default);
            client.call(
                "history",
                "add",
                json!({ "command": command, "cwd": cwd, "pane_id": pane }),
            )
        }
        _ => usage(),
    };

    match result {
        Ok(value) => println!(
            "{}",
            serde_json::to_string_pretty(&value).unwrap_or_else(|_| "null".into())
        ),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
