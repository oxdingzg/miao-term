//! A deliberately minimal, opt-in vim mode (ADR 0029), shared by both hosts.
//!
//! The editing model is a set of pure functions over `(text, pos, pending)`;
//! [`vim_handle`] is the egui glue: in Normal mode it owns the buffer (egui's
//! own edits are discarded and the caret is placed from our state), Insert mode
//! is ordinary editing, and Escape returns to Normal.

/// Live state for the minimal vim mode.
#[derive(Default)]
pub struct VimRuntime {
    pub mode: VimMode,
    pub pending: Option<char>,
    pub pos: usize,
    pub snapshot: String,
    pub undo: Vec<String>,
}

/// Minimal vim editing model (ADR 0029). Opt-in via `editor-vim`.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum VimMode {
    Insert,
    #[default]
    Normal,
}

/// What a normal-mode key did.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum VimEffect {
    Nothing,
    Edit,
    Save,
    Quit,
}

/// Map an egui key to the character vim cares about.
pub fn vim_key_char(key: egui::Key, shift: bool) -> Option<char> {
    use egui::Key;
    Some(match key {
        Key::A => 'a',
        Key::B => 'b',
        Key::C => 'c',
        Key::D => 'd',
        Key::E => 'e',
        Key::F => 'f',
        Key::G => 'g',
        Key::H => 'h',
        Key::I => 'i',
        Key::J => 'j',
        Key::K => 'k',
        Key::L => 'l',
        Key::M => 'm',
        Key::N => 'n',
        Key::O => 'o',
        Key::P => 'p',
        Key::Q => 'q',
        Key::R => 'r',
        Key::S => 's',
        Key::T => 't',
        Key::U => 'u',
        Key::V => 'v',
        Key::W => 'w',
        Key::X => 'x',
        Key::Y => 'y',
        Key::Z => 'z',
        Key::Num0 => '0',
        Key::Num4 if shift => '$',
        Key::Semicolon if shift => ':',
        _ => return None,
    })
}

/// Vim handling for one frame: in Normal mode we own the buffer (egui's own
/// edits are discarded and the caret is placed from our state); Insert mode is
/// ordinary editing, and Escape returns to Normal.
pub fn vim_handle(
    text: &mut String,
    vim: &mut VimRuntime,
    ctx: &egui::Context,
    text_id: egui::Id,
) -> VimEffect {
    use egui::text::{CCursor, CCursorRange};
    let events = ctx.input(|i| i.events.clone());
    let mut state = egui::text_edit::TextEditState::load(ctx, text_id).unwrap_or_default();
    let mut effect = VimEffect::Nothing;

    if vim.mode == VimMode::Insert {
        let escaped = events.iter().any(|e| {
            matches!(
                e,
                egui::Event::Key {
                    key: egui::Key::Escape,
                    pressed: true,
                    ..
                }
            )
        });
        if escaped {
            vim.pos = state
                .cursor
                .char_range()
                .map(|r| r.primary.index)
                .unwrap_or(0);
            vim.snapshot = text.clone();
            vim.mode = VimMode::Normal;
        }
        return VimEffect::Nothing;
    }

    *text = vim.snapshot.clone();
    for event in events {
        let ch = match &event {
            egui::Event::Text(t) => t.chars().next(),
            egui::Event::Key {
                key: egui::Key::Escape,
                pressed: true,
                ..
            } => Some('\u{1b}'),
            egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => vim_key_char(*key, modifiers.shift),
            _ => None,
        };
        let Some(ch) = ch else { continue };
        if ch == '\u{1b}' {
            vim.pending = None;
            continue;
        }
        let before = text.clone();
        let result = if matches!(ch, 'i' | 'a' | 'I' | 'A' | 'o' | 'O') {
            let r = vim_enter_insert(text, &mut vim.pos, ch);
            vim.mode = VimMode::Insert;
            r
        } else if ch == 'u' {
            match vim.undo.pop() {
                Some(prev) => {
                    *text = prev;
                    vim.pos = vim.pos.min(text.chars().count());
                    VimEffect::Edit
                }
                None => VimEffect::Nothing,
            }
        } else {
            vim_key(text, &mut vim.pos, &mut vim.pending, ch)
        };
        if result == VimEffect::Edit {
            vim.undo.push(before);
            vim.snapshot = text.clone();
        }
        if result != VimEffect::Nothing {
            effect = result;
        }
        if result == VimEffect::Quit || result == VimEffect::Save {
            return result;
        }
        if vim.mode == VimMode::Insert {
            state
                .cursor
                .set_char_range(Some(CCursorRange::one(CCursor::new(vim.pos))));
            state.store(ctx, text_id);
            return effect;
        }
    }
    vim.pos = vim.pos.min(vim.snapshot.chars().count());
    state
        .cursor
        .set_char_range(Some(CCursorRange::one(CCursor::new(vim.pos))));
    state.store(ctx, text_id);
    effect
}

fn vim_clamp(pos: usize, len: usize) -> usize {
    pos.min(len)
}

/// Char index of the start of the line containing `pos`.
fn vim_line_start(chars: &[char], pos: usize) -> usize {
    chars[..vim_clamp(pos, chars.len())]
        .iter()
        .rposition(|c| *c == '\n')
        .map(|i| i + 1)
        .unwrap_or(0)
}

/// Char index of the end of the line containing `pos` (exclusive).
fn vim_line_end(chars: &[char], pos: usize) -> usize {
    let pos = vim_clamp(pos, chars.len());
    chars[pos..]
        .iter()
        .position(|c| *c == '\n')
        .map(|i| pos + i)
        .unwrap_or(chars.len())
}

fn vim_line_up(chars: &[char], pos: usize) -> usize {
    let start = vim_line_start(chars, pos);
    if start == 0 {
        return 0;
    }
    let col = pos - start;
    let prev_end = start - 1; // the '\n'
    let prev_start = vim_line_start(chars, prev_end);
    (prev_start + col).min(prev_end)
}

fn vim_line_down(chars: &[char], pos: usize) -> usize {
    let start = vim_line_start(chars, pos);
    let col = pos - start;
    let end = vim_line_end(chars, pos);
    if end >= chars.len() {
        return chars.len();
    }
    let next_start = end + 1;
    let next_end = vim_line_end(chars, vim_clamp(next_start, chars.len()));
    (next_start + col).min(next_end)
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn vim_next_word(chars: &[char], pos: usize) -> usize {
    let len = chars.len();
    let mut i = vim_clamp(pos, len);
    while i < len && is_word_char(chars[i]) {
        i += 1;
    }
    while i < len && !is_word_char(chars[i]) {
        i += 1;
    }
    i
}

fn vim_prev_word(chars: &[char], pos: usize) -> usize {
    let mut i = vim_clamp(pos, chars.len());
    if i == 0 {
        return 0;
    }
    i -= 1;
    while i > 0 && !is_word_char(chars[i]) {
        i -= 1;
    }
    while i > 0 && is_word_char(chars[i - 1]) {
        i -= 1;
    }
    i
}

/// Apply one key in normal mode. `pos` is a char index; `pending` holds a
/// half-typed command (`d`, `g`, `:`).
pub fn vim_key(
    text: &mut String,
    pos: &mut usize,
    pending: &mut Option<char>,
    ch: char,
) -> VimEffect {
    let mut chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut changed = false;

    if let Some(p) = pending.take() {
        match (p, ch) {
            ('d', 'd') => {
                let start = vim_line_start(&chars, *pos);
                let end = vim_line_end(&chars, *pos);
                let end = if end < len { end + 1 } else { end }; // include the '\n'
                chars.drain(start..end);
                *pos = vim_clamp(start, chars.len());
                changed = true;
            }
            ('g', 'g') => *pos = 0,
            (':', 'w') => return VimEffect::Save,
            (':', 'q') => return VimEffect::Quit,
            _ => {}
        }
    } else {
        match ch {
            'h' => *pos = pos.saturating_sub(1),
            'l' => *pos = vim_clamp(*pos + 1, len),
            'j' => *pos = vim_line_down(&chars, *pos),
            'k' => *pos = vim_line_up(&chars, *pos),
            '0' => *pos = vim_line_start(&chars, *pos),
            '$' => *pos = vim_line_end(&chars, *pos),
            'w' => *pos = vim_next_word(&chars, *pos),
            'b' => *pos = vim_prev_word(&chars, *pos),
            'G' => *pos = len,
            'x' => {
                if *pos < len {
                    chars.remove(*pos);
                    changed = true;
                }
            }
            'd' | 'g' | ':' => {
                *pending = Some(ch);
                return VimEffect::Nothing;
            }
            _ => return VimEffect::Nothing,
        }
    }

    if changed {
        *text = chars.into_iter().collect();
        VimEffect::Edit
    } else {
        VimEffect::Nothing
    }
}

/// Insert-mode entry: which key put us here, and any text change to make first.
/// Returns the effect (`Edit` when the buffer changed).
pub fn vim_enter_insert(text: &mut String, pos: &mut usize, ch: char) -> VimEffect {
    let mut chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut changed = false;
    match ch {
        'i' => {}
        'a' => *pos = vim_clamp(*pos + 1, len),
        'I' => *pos = vim_line_start(&chars, *pos),
        'A' => *pos = vim_line_end(&chars, *pos),
        'o' => {
            let end = vim_line_end(&chars, *pos);
            chars.insert(end, '\n');
            *pos = end + 1;
            changed = true;
        }
        'O' => {
            let start = vim_line_start(&chars, *pos);
            chars.insert(start, '\n');
            *pos = start;
            changed = true;
        }
        _ => return VimEffect::Nothing,
    }
    if changed {
        *text = chars.into_iter().collect();
        VimEffect::Edit
    } else {
        VimEffect::Nothing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vim_movement_and_edits() {
        let mut text = "hello world\nsecond line\n".to_string();
        let mut pos = 0usize;
        let mut pending = None;
        // l h 0 $ word motions
        assert_eq!(
            vim_key(&mut text, &mut pos, &mut pending, 'l'),
            VimEffect::Nothing
        );
        assert_eq!(pos, 1);
        vim_key(&mut text, &mut pos, &mut pending, '$');
        assert_eq!(pos, 11);
        vim_key(&mut text, &mut pos, &mut pending, '0');
        assert_eq!(pos, 0);
        vim_key(&mut text, &mut pos, &mut pending, 'w');
        assert_eq!(pos, 6, "start of the next word");
        vim_key(&mut text, &mut pos, &mut pending, 'b');
        assert_eq!(pos, 0);
        // j / k keep the column
        pos = 6;
        vim_key(&mut text, &mut pos, &mut pending, 'j');
        assert_eq!(pos, 18, "same column on the next line");
        vim_key(&mut text, &mut pos, &mut pending, 'k');
        assert_eq!(pos, 6);
        // x deletes the character under the cursor
        assert_eq!(
            vim_key(&mut text, &mut pos, &mut pending, 'x'),
            VimEffect::Edit
        );
        assert_eq!(text, "hello orld\nsecond line\n");
        // dd removes the whole line (including its newline)
        let mut t2 = "one\ntwo\nthree".to_string();
        let mut p2 = 4;
        pending = None;
        assert_eq!(
            vim_key(&mut t2, &mut p2, &mut pending, 'd'),
            VimEffect::Nothing
        );
        assert_eq!(pending, Some('d'));
        assert_eq!(
            vim_key(&mut t2, &mut p2, &mut pending, 'd'),
            VimEffect::Edit
        );
        assert_eq!(t2, "one\nthree");
        // gg / G
        let mut t3 = "a\nb\nc".to_string();
        let mut p3 = 4;
        pending = None;
        vim_key(&mut t3, &mut p3, &mut pending, 'G');
        assert_eq!(p3, 5);
        vim_key(&mut t3, &mut p3, &mut pending, 'g');
        assert_eq!(pending, Some('g'));
        vim_key(&mut t3, &mut p3, &mut pending, 'g');
        assert_eq!(p3, 0);
    }

    #[test]
    fn vim_insert_entry_and_commands() {
        let mut text = "abc".to_string();
        let mut pos = 1;
        // a: append after the cursor
        vim_enter_insert(&mut text, &mut pos, 'a');
        assert_eq!(pos, 2);
        // A: end of line; I: start of line
        vim_enter_insert(&mut text, &mut pos, 'A');
        assert_eq!(pos, 3);
        vim_enter_insert(&mut text, &mut pos, 'I');
        assert_eq!(pos, 0);
        // o / O open lines
        let mut t = "x".to_string();
        let mut p = 0;
        assert_eq!(vim_enter_insert(&mut t, &mut p, 'o'), VimEffect::Edit);
        assert_eq!(t, "x\n");
        assert_eq!(p, 2);
        assert_eq!(vim_enter_insert(&mut t, &mut p, 'O'), VimEffect::Edit);
        assert_eq!(t, "x\n\n");
        // :w and :q
        let mut t2 = "z".to_string();
        let mut p2 = 0;
        let mut pending2 = None;
        vim_key(&mut t2, &mut p2, &mut pending2, ':');
        assert_eq!(pending2, Some(':'));
        assert_eq!(
            vim_key(&mut t2, &mut p2, &mut pending2, 'w'),
            VimEffect::Save
        );
        assert_eq!(
            vim_key(&mut t2, &mut p2, &mut pending2, ':'),
            VimEffect::Nothing
        );
        assert_eq!(
            vim_key(&mut t2, &mut p2, &mut pending2, 'q'),
            VimEffect::Quit
        );
    }

    #[test]
    fn vim_keys_map_from_egui_keys() {
        assert_eq!(vim_key_char(egui::Key::H, false), Some('h'));
        assert_eq!(vim_key_char(egui::Key::Num4, true), Some('$'));
        assert_eq!(vim_key_char(egui::Key::Semicolon, true), Some(':'));
        assert_eq!(vim_key_char(egui::Key::Num4, false), None);
        assert_eq!(vim_key_char(egui::Key::F5, false), None);
    }
}
