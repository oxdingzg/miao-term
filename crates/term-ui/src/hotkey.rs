//! A system-wide hotkey that toggles the Quick Terminal (ADR 0019).
//!
//! macOS and Windows get a real global grab via `global-hotkey` (MIT); other
//! platforms return `None` and the feature is driven by an external binding tool
//! (see `integration::hotkey_snippet`). The accelerator string is parsed here so
//! the parsing is platform-independent and tested everywhere.

/// A parsed accelerator: which modifiers plus a key name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Combo {
    pub cmd: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Canonical key name: `a`..`z`, `0`..`9`, `space`, `grave`, `enter`.
    pub key: String,
}

/// Parse e.g. `cmd+shift+t`, `ctrl+alt+space`. Requires at least one modifier.
pub fn parse(spec: &str) -> Option<Combo> {
    let mut combo = Combo {
        cmd: false,
        ctrl: false,
        alt: false,
        shift: false,
        key: String::new(),
    };
    for part in spec.split('+') {
        match part.trim().to_ascii_lowercase().as_str() {
            "" => {}
            "cmd" | "command" | "super" | "meta" => combo.cmd = true,
            "ctrl" | "control" => combo.ctrl = true,
            "alt" | "option" => combo.alt = true,
            "shift" => combo.shift = true,
            key => combo.key = key.to_string(),
        }
    }
    if combo.key.is_empty() || !(combo.cmd || combo.ctrl || combo.alt) {
        return None;
    }
    if !valid_key(&combo.key) {
        return None;
    }
    Some(combo)
}

fn valid_key(key: &str) -> bool {
    matches!(key, "space" | "grave" | "enter")
        || (key.len() == 1 && key.chars().all(|c| c.is_ascii_alphanumeric()))
}

/// A registered hotkey. `take_pending` reports presses since the last call.
pub struct Hotkeys(imp::Hotkeys);

impl Hotkeys {
    /// Register the accelerator; `wake` is called from the hotkey handler to
    /// repaint the UI. Returns `None` when unsupported or registration fails.
    pub fn register(spec: &str, wake: impl Fn() + Send + Sync + 'static) -> Option<Self> {
        let combo = parse(spec)?;
        imp::Hotkeys::register(&combo, wake).map(Hotkeys)
    }

    pub fn take_pending(&self) -> bool {
        self.0.take_pending()
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod imp {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use global_hotkey::hotkey::{Code, HotKey, Modifiers};
    use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

    use super::Combo;

    pub struct Hotkeys {
        _manager: GlobalHotKeyManager,
        pending: Arc<AtomicBool>,
    }

    impl Hotkeys {
        pub fn register(combo: &Combo, wake: impl Fn() + Send + Sync + 'static) -> Option<Self> {
            let manager = GlobalHotKeyManager::new().ok()?;
            let hotkey = HotKey::new(Some(modifiers(combo)), code(&combo.key)?);
            manager.register(hotkey).ok()?;
            let pending = Arc::new(AtomicBool::new(false));
            let flag = pending.clone();
            GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
                if event.state == HotKeyState::Pressed {
                    flag.store(true, Ordering::SeqCst);
                    wake();
                }
            }));
            Some(Self {
                _manager: manager,
                pending,
            })
        }

        pub fn take_pending(&self) -> bool {
            self.pending.swap(false, Ordering::SeqCst)
        }
    }

    fn modifiers(combo: &Combo) -> Modifiers {
        let mut m = Modifiers::empty();
        if combo.cmd {
            m |= Modifiers::SUPER;
        }
        if combo.ctrl {
            m |= Modifiers::CONTROL;
        }
        if combo.alt {
            m |= Modifiers::ALT;
        }
        if combo.shift {
            m |= Modifiers::SHIFT;
        }
        m
    }

    fn code(key: &str) -> Option<Code> {
        match key {
            "space" => Some(Code::Space),
            "grave" => Some(Code::Backquote),
            "enter" => Some(Code::Enter),
            k if k.len() == 1 => {
                let c = k.chars().next()?;
                if c.is_ascii_alphabetic() {
                    // `Code::KeyA` … `Code::KeyZ`
                    let name = format!("Key{}", c.to_ascii_uppercase());
                    named_code(&name)
                } else if c.is_ascii_digit() {
                    let name = format!("Digit{c}");
                    named_code(&name)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Map a `Code` variant name without a giant match (uses its Debug name).
    fn named_code(name: &str) -> Option<Code> {
        // `Code` is a fieldless enum; match the plausible keys explicitly.
        const LETTERS: [(u8, Code); 26] = [
            (b'A', Code::KeyA),
            (b'B', Code::KeyB),
            (b'C', Code::KeyC),
            (b'D', Code::KeyD),
            (b'E', Code::KeyE),
            (b'F', Code::KeyF),
            (b'G', Code::KeyG),
            (b'H', Code::KeyH),
            (b'I', Code::KeyI),
            (b'J', Code::KeyJ),
            (b'K', Code::KeyK),
            (b'L', Code::KeyL),
            (b'M', Code::KeyM),
            (b'N', Code::KeyN),
            (b'O', Code::KeyO),
            (b'P', Code::KeyP),
            (b'Q', Code::KeyQ),
            (b'R', Code::KeyR),
            (b'S', Code::KeyS),
            (b'T', Code::KeyT),
            (b'U', Code::KeyU),
            (b'V', Code::KeyV),
            (b'W', Code::KeyW),
            (b'X', Code::KeyX),
            (b'Y', Code::KeyY),
            (b'Z', Code::KeyZ),
        ];
        const DIGITS: [(u8, Code); 10] = [
            (b'0', Code::Digit0),
            (b'1', Code::Digit1),
            (b'2', Code::Digit2),
            (b'3', Code::Digit3),
            (b'4', Code::Digit4),
            (b'5', Code::Digit5),
            (b'6', Code::Digit6),
            (b'7', Code::Digit7),
            (b'8', Code::Digit8),
            (b'9', Code::Digit9),
        ];
        if let Some(rest) = name.strip_prefix("Key") {
            let c = rest.as_bytes().first().copied()?;
            return LETTERS.iter().find(|(k, _)| *k == c).map(|(_, v)| *v);
        }
        if let Some(rest) = name.strip_prefix("Digit") {
            let c = rest.as_bytes().first().copied()?;
            return DIGITS.iter().find(|(k, _)| *k == c).map(|(_, v)| *v);
        }
        None
    }
}

#[cfg(target_os = "linux")]
mod imp {
    //! Wayland/X11 global shortcut via the `GlobalShortcuts` XDG portal.
    //!
    //! The portal (not us) owns the trigger: it may show a dialog and lets the
    //! user pick or change the key. We bind one shortcut id, `quick`, and flip
    //! the pending flag whenever the compositor reports it activated.

    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
    use ashpd::zbus::export::futures_core::Stream;

    use super::Combo;

    pub struct Hotkeys {
        pending: Arc<AtomicBool>,
    }

    impl Hotkeys {
        pub fn register(_combo: &Combo, wake: impl Fn() + Send + Sync + 'static) -> Option<Self> {
            let pending = Arc::new(AtomicBool::new(false));
            let flag = pending.clone();
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .ok()?;
            std::thread::spawn(move || {
                let _ = runtime.block_on(async move {
                    let shortcuts = GlobalShortcuts::new().await.ok()?;
                    let session = shortcuts.create_session().await.ok()?;
                    let request = shortcuts
                        .bind_shortcuts(
                            &session,
                            &[NewShortcut::new("quick", "Quick Terminal")],
                            None,
                        )
                        .await
                        .ok()?;
                    let _ = request.response().ok()?;
                    let mut activated = shortcuts.receive_activated().await.ok()?;
                    loop {
                        let item =
                            std::future::poll_fn(|cx| Pin::new(&mut activated).poll_next(cx)).await;
                        match item {
                            Some(event) if event.shortcut_id() == "quick" => {
                                flag.store(true, Ordering::SeqCst);
                                wake();
                            }
                            Some(_) => {}
                            None => break,
                        }
                    }
                    Some(())
                });
            });
            Some(Self { pending })
        }

        pub fn take_pending(&self) -> bool {
            self.pending.swap(false, Ordering::SeqCst)
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
mod imp {
    use super::Combo;

    pub struct Hotkeys;

    impl Hotkeys {
        pub fn register(_combo: &Combo, _wake: impl Fn() + Send + Sync + 'static) -> Option<Self> {
            None
        }

        pub fn take_pending(&self) -> bool {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_accelerators() {
        let c = parse("cmd+shift+t").unwrap();
        assert!(c.cmd && c.shift && !c.ctrl && !c.alt);
        assert_eq!(c.key, "t");
        let c = parse("Ctrl+Alt+Space").unwrap();
        assert!(c.ctrl && c.alt);
        assert_eq!(c.key, "space");
        let c = parse("super+grave").unwrap();
        assert!(c.cmd);
        assert_eq!(c.key, "grave");
    }

    #[test]
    fn rejects_invalid_specs() {
        assert!(parse("t").is_none(), "needs a modifier");
        assert!(parse("shift+t").is_none(), "shift alone is not enough");
        assert!(parse("cmd+").is_none(), "needs a key");
        assert!(parse("cmd+shift+€").is_none(), "unknown key");
    }
}
