//! Files dragged onto the window: insert their paths or open them.
//!
//! Over a terminal pane a drop inserts the shell-quoted paths (what a shell
//! command or an agent prompt wants), except on a band along the pane's
//! bottom, which opens them instead: files in the editor, a folder as a new
//! terminal there. Both targets are drawn while dragging, so which one is
//! about to happen is visible. Holding Alt (⌥) swaps them. Elsewhere a drop
//! opens.
//!
//! The window gets no pointer motion and no key events while the OS drags,
//! so the pointer and Alt are read from the system: live on macOS, Windows
//! and Wayland (its own drag events). Where that is not possible (X11) the
//! band is not offered and the drop inserts unless Alt is held.

use std::path::PathBuf;

use miao_term_ui::i18n::{t, Lang};
use miao_term_ui::Rect;

/// What dropping does at a point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropAction {
    /// Paste the shell-quoted paths into the terminal under the pointer.
    InsertPath,
    /// Files open in the editor; a folder opens a terminal there.
    Open,
}

impl DropAction {
    fn flipped(self) -> Self {
        match self {
            DropAction::InsertPath => DropAction::Open,
            DropAction::Open => DropAction::InsertPath,
        }
    }
}

/// Where a terminal pane's "open" band starts: its bottom quarter, at least
/// 44 points tall and at most half the pane.
pub fn open_band_top(pane: Rect) -> f32 {
    let band = (pane.h * 0.25).max(44.0).min(pane.h * 0.5);
    pane.y + pane.h - band
}

/// The action for a drop at height `y` (logical points). `pane` is the
/// terminal pane under the pointer, if any; `live` says whether the pointer
/// position is known during the drag (else the band is not offered).
pub fn drop_action(pane: Option<Rect>, y: f32, live: bool, alt: bool) -> DropAction {
    let action = match pane {
        Some(r) if live && y >= open_band_top(r) => DropAction::Open,
        Some(_) => DropAction::InsertPath,
        // Off a terminal: open (the editor area, the background).
        None => return DropAction::Open,
    };
    if alt {
        action.flipped()
    } else {
        action
    }
}

/// The two targets' labels for what is being dragged (unknown on Wayland,
/// whose drag events carry no paths until the drop).
pub fn labels(lang: Lang, paths: &[PathBuf]) -> (String, String) {
    let n = paths.len();
    let dirs = paths.iter().filter(|p| p.is_dir()).count();
    let insert = if n > 1 {
        match lang {
            Lang::Zh => format!("插入 {n} 个路径"),
            _ => format!("Insert {n} paths"),
        }
    } else {
        t(lang, "Insert path", "插入路径").to_string()
    };
    let open = match (n, dirs) {
        (0, _) => t(lang, "Open in mtty", "在 mtty 中打开").to_string(),
        (1, 1) => t(lang, "Open a terminal here", "在此打开新终端").to_string(),
        (1, _) => t(lang, "Open in the editor", "在编辑器中打开").to_string(),
        (n, d) if d == n => match lang {
            Lang::Zh => format!("打开 {n} 个终端"),
            _ => format!("Open {n} terminals"),
        },
        (n, 0) => match lang {
            Lang::Zh => format!("在编辑器中打开 {n} 个文件"),
            _ => format!("Open {n} files in the editor"),
        },
        (n, _) => match lang {
            Lang::Zh => format!("打开 {n} 项"),
            _ => format!("Open {n} items"),
        },
    };
    (insert, open)
}

/// The hint under the labels.
pub fn hint(lang: Lang) -> &'static str {
    if cfg!(target_os = "macos") {
        // Spelled out: the UI font has no ⌥ glyph.
        t(lang, "Hold Option to swap", "按住 Option 切换")
    } else {
        t(lang, "Hold Alt to swap", "按住 Alt 切换")
    }
}

/// The pointer in window pixels, read from the system during a drag, or
/// `None` where that cannot be done.
pub fn pointer_in_window(window: &winit::window::Window) -> Option<(f64, f64)> {
    #[cfg(target_os = "macos")]
    {
        // Screen points from the top left; the window origin is in pixels.
        let (x, y) = crate::macos_url::pointer_on_screen()?;
        let inner = window.inner_position().ok()?;
        let scale = window.scale_factor();
        Some((x * scale - inner.x as f64, y * scale - inner.y as f64))
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
        let mut at = POINT { x: 0, y: 0 };
        // SAFETY: GetCursorPos writes the point on success.
        if unsafe { GetCursorPos(&mut at) } == 0 {
            return None;
        }
        // Both in physical screen pixels (the process is DPI aware).
        let inner = window.inner_position().ok()?;
        Some(((at.x - inner.x) as f64, (at.y - inner.y) as f64))
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = window;
        None
    }
}

/// Whether Alt (⌥) is down now. `known` is the last state the window saw,
/// used where the system cannot be asked.
pub fn alt_down(known: winit::keyboard::ModifiersState) -> bool {
    #[cfg(target_os = "macos")]
    {
        let _ = known;
        crate::macos_url::option_key_down()
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_MENU};
        let _ = known;
        // SAFETY: plain query of the key state.
        unsafe { GetKeyState(i32::from(VK_MENU)) < 0 }
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        known.alt_key()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PANE: Rect = Rect {
        x: 0.0,
        y: 100.0,
        w: 600.0,
        h: 400.0,
    };

    #[test]
    fn a_terminal_inserts_except_on_its_bottom_band() {
        assert_eq!(open_band_top(PANE), 400.0, "the bottom quarter");
        assert_eq!(
            drop_action(Some(PANE), 150.0, true, false),
            DropAction::InsertPath
        );
        assert_eq!(
            drop_action(Some(PANE), 399.0, true, false),
            DropAction::InsertPath
        );
        assert_eq!(
            drop_action(Some(PANE), 420.0, true, false),
            DropAction::Open
        );
        // Alt swaps them.
        assert_eq!(drop_action(Some(PANE), 150.0, true, true), DropAction::Open);
        assert_eq!(
            drop_action(Some(PANE), 420.0, true, true),
            DropAction::InsertPath
        );
        // Off a terminal a drop opens, Alt or not.
        assert_eq!(drop_action(None, 150.0, true, false), DropAction::Open);
        assert_eq!(drop_action(None, 150.0, true, true), DropAction::Open);
    }

    #[test]
    fn without_a_live_pointer_the_band_is_not_offered() {
        assert_eq!(
            drop_action(Some(PANE), 420.0, false, false),
            DropAction::InsertPath
        );
        assert_eq!(
            drop_action(Some(PANE), 420.0, false, true),
            DropAction::Open
        );
    }

    #[test]
    fn the_band_stays_usable_on_small_and_large_panes() {
        let small = Rect { h: 60.0, ..PANE };
        assert_eq!(open_band_top(small), 100.0 + 60.0 - 30.0, "at most half");
        let mid = Rect { h: 160.0, ..PANE };
        assert_eq!(
            open_band_top(mid),
            100.0 + 160.0 - 44.0,
            "at least 44 points"
        );
    }

    #[test]
    fn labels_follow_what_is_dragged() {
        let dir = std::env::temp_dir();
        let file = dir.join("mtty-drag-label-test.txt");
        std::fs::write(&file, b"x").unwrap();
        let en = |paths: &[PathBuf]| labels(Lang::En, paths);
        assert_eq!(en(&[]).1, "Open in mtty");
        assert_eq!(
            en(std::slice::from_ref(&file)),
            ("Insert path".into(), "Open in the editor".into())
        );
        assert_eq!(en(std::slice::from_ref(&dir)).1, "Open a terminal here");
        assert_eq!(
            en(&[file.clone(), file.clone()]),
            ("Insert 2 paths".into(), "Open 2 files in the editor".into())
        );
        assert_eq!(en(&[dir.clone(), dir.clone()]).1, "Open 2 terminals");
        assert_eq!(en(&[dir.clone(), file.clone()]).1, "Open 2 items");
        assert_eq!(labels(Lang::Zh, std::slice::from_ref(&file)).0, "插入路径");
        let _ = std::fs::remove_file(&file);
    }
}
