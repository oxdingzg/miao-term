//! Host-agnostic key encoding: a neutral key + modifiers → PTY bytes.
//!
//! Each host maps its own event type (winit `KeyEvent`, egui events) into
//! [`KeyKind`]/[`Modifiers`] and calls [`encode_key`], so the encoding rules —
//! control bytes, modifier-aware navigation, Backspace variants, and the kitty
//! keyboard protocol (CSI-u) — live in exactly one place.

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Command (macOS ⌘) / Super / Win. Reserved for host shortcuts.
    pub sup: bool,
}

impl Modifiers {
    /// xterm modifier parameter: 1 + Shift(1) + Alt(2) + Ctrl(4).
    fn number(self) -> u8 {
        1 + u8::from(self.shift) + 2 * u8::from(self.alt) + 4 * u8::from(self.ctrl)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    Char(char),
    Enter,
    Backspace,
    Tab,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    Delete,
    PageUp,
    PageDown,
    Other,
}

/// Escape sequence options for a key press.
#[derive(Clone, Copy, Default)]
pub struct EncodeOpts {
    pub app_cursor: bool,
    pub bracketed: bool,
    pub kitty: bool,
    pub has_selection: bool,
}

/// Plain text (a typed character / IME commit).
pub fn encode_text(text: &str) -> Vec<u8> {
    text.as_bytes().to_vec()
}

/// Paste, honoring bracketed-paste mode and normalizing newlines to CR.
pub fn encode_paste(text: &str, bracketed: bool) -> Vec<u8> {
    let body = text.replace('\n', "\r");
    if bracketed {
        format!("\x1b[200~{body}\x1b[201~").into_bytes()
    } else {
        body.into_bytes()
    }
}

/// Encode a key press (special keys and modified keys; printable characters
/// arrive through [`encode_text`]).
pub fn encode_key(kind: KeyKind, mods: Modifiers, opts: EncodeOpts) -> Vec<u8> {
    // Cmd/Super is reserved for host shortcuts; never sent to the shell.
    if mods.sup {
        return Vec::new();
    }
    let mut out = Vec::new();

    // Kitty keyboard protocol: disambiguate special keys and Ctrl+keys as CSI-u.
    if opts.kitty {
        let n = mods.number();
        let special = match kind {
            KeyKind::Escape => Some(27u32),
            KeyKind::Enter => Some(13),
            KeyKind::Tab => Some(9),
            KeyKind::Backspace => Some(127),
            _ => None,
        };
        if let Some(cp) = special {
            if n > 1 {
                return format!("\x1b[{cp};{n}u").into_bytes();
            }
        }
        if mods.ctrl {
            if let KeyKind::Char(c) = kind {
                return format!("\x1b[{};{n}u", c as u32).into_bytes();
            }
        }
    }

    // Ctrl combos (Unix control bytes). Ctrl+C with a selection is a copy.
    if mods.ctrl {
        if let KeyKind::Char(c) = kind {
            let c = c.to_ascii_uppercase();
            if ('@'..='_').contains(&c) {
                if c == 'C' && opts.has_selection {
                    return Vec::new();
                }
                out.push(c as u8 - 0x40);
                return out;
            }
        }
    }

    // Backspace variants: Ctrl+Backspace = ^W, Alt+Backspace = ESC DEL.
    if kind == KeyKind::Backspace {
        if mods.ctrl && !mods.alt {
            out.push(0x17);
            return out;
        }
        if mods.alt && !mods.ctrl {
            out.extend_from_slice(b"\x1b\x7f");
            return out;
        }
    }

    // Modifier-aware navigation (word movement, selection, …).
    if mods.shift || mods.alt || mods.ctrl {
        let n = mods.number();
        let seq = match kind {
            KeyKind::Up => Some(format!("\x1b[1;{n}A")),
            KeyKind::Down => Some(format!("\x1b[1;{n}B")),
            KeyKind::Right => Some(format!("\x1b[1;{n}C")),
            KeyKind::Left => Some(format!("\x1b[1;{n}D")),
            KeyKind::Home => Some(format!("\x1b[1;{n}H")),
            KeyKind::End => Some(format!("\x1b[1;{n}F")),
            KeyKind::Delete => Some(format!("\x1b[3;{n}~")),
            KeyKind::PageUp => Some(format!("\x1b[5;{n}~")),
            KeyKind::PageDown => Some(format!("\x1b[6;{n}~")),
            _ => None,
        };
        if let Some(seq) = seq {
            return seq.into_bytes();
        }
    }

    match kind {
        KeyKind::Enter => out.push(b'\r'),
        KeyKind::Backspace => out.push(0x7f),
        KeyKind::Tab => {
            if mods.shift {
                out.extend_from_slice(b"\x1b[Z");
            } else {
                out.push(b'\t');
            }
        }
        KeyKind::Escape => out.push(0x1b),
        KeyKind::Up => cursor(&mut out, b'A', opts.app_cursor),
        KeyKind::Down => cursor(&mut out, b'B', opts.app_cursor),
        KeyKind::Right => cursor(&mut out, b'C', opts.app_cursor),
        KeyKind::Left => cursor(&mut out, b'D', opts.app_cursor),
        KeyKind::Home => out.extend_from_slice(b"\x1b[H"),
        KeyKind::End => out.extend_from_slice(b"\x1b[F"),
        KeyKind::Delete => out.extend_from_slice(b"\x1b[3~"),
        KeyKind::PageUp => out.extend_from_slice(b"\x1b[5~"),
        KeyKind::PageDown => out.extend_from_slice(b"\x1b[6~"),
        KeyKind::Char(_) | KeyKind::Other => {}
    }
    out
}

fn cursor(out: &mut Vec<u8>, c: u8, app_cursor: bool) {
    if app_cursor {
        out.extend_from_slice(&[0x1b, b'O', c]);
    } else {
        out.extend_from_slice(&[0x1b, b'[', c]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_letters_are_control_bytes() {
        let m = Modifiers {
            ctrl: true,
            ..Default::default()
        };
        assert_eq!(
            encode_key(KeyKind::Char('a'), m, EncodeOpts::default()),
            vec![1]
        );
        assert_eq!(
            encode_key(KeyKind::Char('c'), m, EncodeOpts::default()),
            vec![3]
        );
    }

    #[test]
    fn cmd_is_not_sent() {
        let m = Modifiers {
            sup: true,
            ..Default::default()
        };
        assert!(encode_key(KeyKind::Char('a'), m, EncodeOpts::default()).is_empty());
    }

    #[test]
    fn shift_tab_and_arrows() {
        let shift = Modifiers {
            shift: true,
            ..Default::default()
        };
        assert_eq!(
            encode_key(KeyKind::Tab, shift, EncodeOpts::default()),
            b"\x1b[Z"
        );
        let ctrl = Modifiers {
            ctrl: true,
            ..Default::default()
        };
        assert_eq!(
            encode_key(KeyKind::Right, ctrl, EncodeOpts::default()),
            b"\x1b[1;5C"
        );
    }

    #[test]
    fn paste_normalizes_newlines() {
        assert_eq!(encode_paste("a\nb", false), b"a\rb");
        assert_eq!(encode_paste("a", true), b"\x1b[200~a\x1b[201~");
    }
}
