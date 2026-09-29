//! Host-agnostic key encoding: a neutral key + modifiers → PTY bytes.
//!
//! Each host maps its own event type (winit `KeyEvent`, egui events) into
//! [`KeyInput`] and calls [`encode`], so the encoding rules live in one place.

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub sup: bool,
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

#[derive(Clone)]
pub struct KeyInput {
    pub kind: KeyKind,
    /// Text produced by the key (layout-aware), if any.
    pub text: Option<String>,
}

impl KeyInput {
    pub fn char(c: char) -> Self {
        Self {
            kind: KeyKind::Char(c),
            text: Some(c.to_string()),
        }
    }
}

/// Encode a key press to the bytes to write to the PTY.
pub fn encode(key: &KeyInput, mods: Modifiers, app_cursor: bool) -> Vec<u8> {
    // Cmd (super) is reserved for host shortcuts; never sent to the shell.
    if mods.sup {
        return Vec::new();
    }
    let mut out = Vec::new();
    if mods.alt {
        out.push(0x1b);
    }
    if let KeyKind::Char(c) = key.kind {
        if mods.ctrl {
            let c = c.to_ascii_uppercase();
            if ('@'..='_').contains(&c) {
                out.push(c as u8 - 0x40);
                return out;
            }
        }
        if let Some(text) = &key.text {
            out.extend_from_slice(text.as_bytes());
        } else {
            let mut buf = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        }
        return out;
    }
    if let Some(text) = &key.text {
        out.extend_from_slice(text.as_bytes());
        return out;
    }
    let seq: &[u8] = match key.kind {
        KeyKind::Enter => b"\r",
        KeyKind::Backspace => &[0x7f],
        KeyKind::Tab => b"\t",
        KeyKind::Escape => &[0x1b],
        KeyKind::Up => cursor(b'A', app_cursor),
        KeyKind::Down => cursor(b'B', app_cursor),
        KeyKind::Right => cursor(b'C', app_cursor),
        KeyKind::Left => cursor(b'D', app_cursor),
        KeyKind::Home => b"\x1b[H",
        KeyKind::End => b"\x1b[F",
        KeyKind::Delete => b"\x1b[3~",
        KeyKind::PageUp => b"\x1b[5~",
        KeyKind::PageDown => b"\x1b[6~",
        _ => return out,
    };
    out.extend_from_slice(seq);
    out
}

fn cursor(final_byte: u8, app_cursor: bool) -> &'static [u8] {
    // 0x1b 'O' c or 0x1b '[' c
    match (app_cursor, final_byte) {
        (true, b'A') => b"\x1bOA",
        (true, b'B') => b"\x1bOB",
        (true, b'C') => b"\x1bOC",
        (true, b'D') => b"\x1bOD",
        (false, b'A') => b"\x1b[A",
        (false, b'B') => b"\x1b[B",
        (false, b'C') => b"\x1b[C",
        _ => b"\x1b[D",
    }
}
