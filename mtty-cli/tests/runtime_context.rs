#![cfg(unix)]

use serde_json::json;
use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn client_runtime_context_is_scoped_to_its_pane_and_can_be_cleared() {
    let path = std::env::temp_dir().join(format!("mtty-context-{}.sock", std::process::id()));
    let state = mtty_mtp::ServerState::new();
    mtty_mtp::serve(&path, state.clone()).unwrap();
    let report = |pane: &str, context: serde_json::Value, session: Option<&str>| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_mtty-cli"));
        command.args([
            "--socket",
            path.to_str().unwrap(),
            "--wait",
            "2",
            "state",
            "miao",
            "--state",
            "idle",
            "--pane",
            pane,
            "--runtime-context",
            "-",
        ]);
        if let Some(session) = session {
            command.args(["--session", session]);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(context.to_string().as_bytes())
            .unwrap();
        assert!(child.wait().unwrap().success());
    };
    let first = json!({"kind":"owned", "runtimeID":"runtime-first", "storage":"first.db"});
    let second = json!({"kind":"owned", "runtimeID":"runtime-second", "storage":"second.db"});
    // Identically named sessions in separate storage must retain separate targets.
    report("pane1", first.clone(), Some("ses_same"));
    report("pane2", second.clone(), Some("ses_same"));
    assert_eq!(state.agent_for("pane1").unwrap()["runtime_context"], first);
    assert_eq!(state.agent_for("pane2").unwrap()["runtime_context"], second);
    report("pane1", json!({"kind":"attached"}), Some("remote_session"));
    assert_eq!(
        state.agent_for("pane1").unwrap()["runtime_context"],
        json!({"kind":"attached"})
    );
    report("pane1", serde_json::Value::Null, None);
    let cleared = state.agent_for("pane1").unwrap();
    assert!(cleared["runtime_context"].is_null());
    assert!(cleared["session_id"].is_null());
    assert_eq!(state.agent_for("pane2").unwrap()["runtime_context"], second);
    let _ = std::fs::remove_file(path);
}
