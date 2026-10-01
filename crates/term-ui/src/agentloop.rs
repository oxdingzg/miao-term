//! Agent-loop side effects: system notifications and sleep prevention
//! (ADR 0010). Both shell out to platform tools and degrade to no-ops where a
//! tool is missing, so nothing here can fail the app.

use std::process::{Child, Command};

/// Post a system notification. Best-effort: failures are ignored.
pub fn notify(title: &str, body: &str) {
    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "display notification \"{}\" with title \"{}\"",
            escape(body),
            escape(title)
        );
        let _ = Command::new("osascript").arg("-e").arg(script).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = Command::new("notify-send").arg(title).arg(body).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        // Best-effort; requires the BurntToast module, which may be absent.
        let script = format!(
            "New-BurntToastNotification -Text '{}','{}'",
            escape_ps(title),
            escape_ps(body)
        );
        let _ = Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .spawn();
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = (title, body);
    }
}

/// Show a blocking error dialog, for failures before any window can draw
/// (an app started from Finder or a launcher has no visible stderr).
/// Best-effort: without a dialog tool it does nothing.
pub fn alert(title: &str, body: &str) {
    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "display alert \"{}\" message \"{}\" as critical",
            escape(title),
            escape(body)
        );
        let _ = Command::new("osascript").arg("-e").arg(script).status();
    }
    #[cfg(target_os = "linux")]
    {
        let text = format!("{title}\n\n{body}");
        let shown = Command::new("zenity")
            .args(["--error", "--no-markup", "--text", &text])
            .status()
            .is_ok_and(|s| s.success());
        if !shown {
            let _ = Command::new("kdialog").args(["--error", &text]).status();
        }
    }
    #[cfg(target_os = "windows")]
    {
        let script = format!(
            "Add-Type -AssemblyName PresentationFramework; \
             [System.Windows.MessageBox]::Show('{}', '{}', 'OK', 'Error') | Out-Null",
            escape_ps(body),
            escape_ps(title)
        );
        let _ = Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .status();
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = (title, body);
    }
}

/// Escape a string for an AppleScript double-quoted literal.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn escape_ps(s: &str) -> String {
    s.replace('\'', "''")
}

/// Keeps the machine awake while an agent is processing. The child process is
/// platform-specific and is killed when no longer needed or on drop.
#[derive(Default)]
pub struct SleepGuard {
    child: Option<Child>,
}

impl SleepGuard {
    pub fn new() -> Self {
        Self::default()
    }

    /// Idempotently start or stop the inhibitor.
    pub fn set_awake(&mut self, awake: bool) {
        match (awake, self.child.is_some()) {
            (true, false) => self.child = spawn_inhibitor(),
            (false, true) => {
                if let Some(mut child) = self.child.take() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
            _ => {}
        }
    }

    pub fn awake(&self) -> bool {
        self.child.is_some()
    }
}

impl Drop for SleepGuard {
    fn drop(&mut self) {
        self.set_awake(false);
    }
}

// Both inhibitors also watch our pid, so they end with mtty even when it
// exits without dropping the guard (process::exit, a crash, a kill).
#[cfg(target_os = "macos")]
fn spawn_inhibitor() -> Option<Child> {
    let pid = std::process::id().to_string();
    Command::new("caffeinate")
        .args(["-dims", "-w", &pid])
        .spawn()
        .ok()
}

#[cfg(target_os = "linux")]
fn spawn_inhibitor() -> Option<Child> {
    Command::new("systemd-inhibit")
        .args([
            "--what=idle:sleep",
            "--why=mtty: agent processing",
            "--mode=block",
            "tail",
            &format!("--pid={}", std::process::id()),
            "-f",
            "/dev/null",
        ])
        .spawn()
        .ok()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn spawn_inhibitor() -> Option<Child> {
    None
}

/// A prompt waiting for an agent (Composer *Queue*, ADR 0010).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct QueuedPrompt {
    pub text: String,
    /// The pane it was queued for. `None` (prompts saved by older versions)
    /// is never delivered automatically, only by hand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<String>,
}

/// The prompt queue: delivered one at a time to a pane when its agent turns
/// idle.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PromptQueue {
    pub items: Vec<QueuedPrompt>,
}

impl PromptQueue {
    pub fn push(&mut self, text: String, pane: Option<String>) {
        if !text.trim().is_empty() {
            self.items.push(QueuedPrompt { text, pane });
        }
    }

    /// An agent state change in `pane`. On a transition into `idle` the
    /// oldest prompt queued for that pane is taken for delivery. Repeated
    /// `idle` reports are not transitions, so nothing is delivered twice.
    pub fn on_state(
        &mut self,
        pane: &str,
        previous: Option<&str>,
        now: &str,
    ) -> Option<QueuedPrompt> {
        if now != "idle" || previous == Some("idle") {
            return None;
        }
        let i = self
            .items
            .iter()
            .position(|p| p.pane.as_deref() == Some(pane))?;
        Some(self.items.remove(i))
    }

    /// Follow pane ids renamed by a session restore (old id -> new id).
    pub fn remap(&mut self, ids: &std::collections::HashMap<String, String>) {
        for item in &mut self.items {
            if let Some(new) = item.pane.as_ref().and_then(|p| ids.get(p)) {
                item.pane = Some(new.clone());
            }
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({ "items": self.items })
    }

    /// Read `{"items": [...]}`, or the older `{"prompts": ["text", ...]}`.
    pub fn from_json(value: &serde_json::Value) -> Self {
        if let Some(items) = value.get("items") {
            let items = serde_json::from_value(items.clone()).unwrap_or_default();
            return Self { items };
        }
        let items = value
            .get("prompts")
            .and_then(|p| p.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str())
                    .map(|text| QueuedPrompt {
                        text: text.to_string(),
                        pane: None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self { items }
    }
}

#[cfg(test)]
mod queue_tests {
    use super::*;

    fn q(items: &[(&str, Option<&str>)]) -> PromptQueue {
        let mut queue = PromptQueue::default();
        for (text, pane) in items {
            queue.push(text.to_string(), pane.map(str::to_string));
        }
        queue
    }

    #[test]
    fn delivers_one_prompt_per_transition_to_idle() {
        let mut queue = q(&[
            ("first", Some("p1")),
            ("other", Some("p2")),
            ("second", Some("p1")),
        ]);
        assert_eq!(queue.on_state("p1", Some("processing"), "processing"), None);
        let got = queue.on_state("p1", Some("processing"), "idle").unwrap();
        assert_eq!(got.text, "first");
        // The same idle reported again is not a new transition.
        assert_eq!(queue.on_state("p1", Some("idle"), "idle"), None);
        assert_eq!(
            queue
                .on_state("p1", Some("processing"), "idle")
                .unwrap()
                .text,
            "second"
        );
        assert_eq!(queue.on_state("p1", Some("processing"), "idle"), None);
        assert_eq!(queue.items.len(), 1, "other panes keep theirs");
        // A freshly started agent that reports idle first also gets its prompt.
        assert_eq!(queue.on_state("p2", None, "idle").unwrap().text, "other");
    }

    #[test]
    fn legacy_prompts_load_but_are_never_auto_delivered() {
        let mut queue = PromptQueue::from_json(&serde_json::json!({ "prompts": ["old"] }));
        assert_eq!(
            queue.items,
            vec![QueuedPrompt {
                text: "old".into(),
                pane: None
            }]
        );
        assert_eq!(queue.on_state("p1", Some("processing"), "idle"), None);
        let round = PromptQueue::from_json(&queue.to_json());
        assert_eq!(round, queue);
    }

    #[test]
    fn restore_remaps_targets() {
        let mut queue = q(&[("x", Some("old-a")), ("y", Some("gone"))]);
        let ids = [("old-a".to_string(), "pane7".to_string())]
            .into_iter()
            .collect();
        queue.remap(&ids);
        assert_eq!(queue.items[0].pane.as_deref(), Some("pane7"));
        assert_eq!(queue.items[1].pane.as_deref(), Some("gone"));
        assert!(
            q(&[("  ", Some("p"))]).items.is_empty(),
            "blank prompts are not queued"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_applescript_strings() {
        assert_eq!(escape("a\"b\\c"), "a\\\"b\\\\c");
    }

    #[test]
    fn sleep_guard_is_idempotent() {
        let mut guard = SleepGuard::new();
        guard.set_awake(false);
        assert!(!guard.awake());
        guard.set_awake(false);
        assert!(!guard.awake());
    }
}
