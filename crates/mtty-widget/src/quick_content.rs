//! Bounded text search behind Open Quickly.
//!
//! The Open Quickly window matches text inside files under the active pane's
//! directory and inside the active pane's scrollback. The file scan runs on a
//! worker thread and is bounded by file count, per-file size, total bytes,
//! wall-clock time and hit count, so a huge tree or a giant file cannot stall
//! the UI or flood the list. Everything here is pure (no window, no IO beyond
//! the paths it is handed) so it can be unit-tested directly.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// One matching line inside a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FileHit {
    pub path: PathBuf,
    /// 1-based line number.
    pub line: usize,
    /// The matching line, trimmed and capped for a one-line label.
    pub text: String,
}

/// One cell of a captured scrollback line: (column, char, width).
pub(crate) type ScrollCell = (u16, char, u16);

/// A captured scrollback line: (absolute buffer line, cells).
pub(crate) type ScrollLine = (usize, Vec<ScrollCell>);

/// One matching line of a pane's scrollback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScrollHit {
    /// 0-based absolute buffer line, counted from the oldest.
    pub line: usize,
    /// Start column and width in cells (wide characters counted correctly).
    pub col: u16,
    pub width: u16,
    /// The matching line, trimmed and capped for a one-line label.
    pub text: String,
}

/// Caps for one file-content scan.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ScanLimits {
    pub max_files: usize,
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
    pub max_hits: usize,
    pub max_depth: usize,
    pub time: Duration,
}

impl Default for ScanLimits {
    fn default() -> Self {
        Self {
            max_files: 4_000,
            max_file_bytes: 1 << 20,
            max_total_bytes: 24 << 20,
            max_hits: 200,
            max_depth: 8,
            time: Duration::from_millis(300),
        }
    }
}

/// Directories never worth scanning: version control, build output and
/// dependency trees dominate a checkout and rarely hold the text people mean.
const SKIP_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "target",
    "node_modules",
    "vendor",
    "dist",
    "build",
    ".cache",
    "__pycache__",
];

/// Most characters a single line keeps in a label; the window clips the rest.
pub(crate) const MAX_LABEL_TEXT: usize = 120;

/// A one-line preview of `s`: trimmed, and capped with an ellipsis.
pub(crate) fn preview(s: &str) -> String {
    let s = s.trim();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i >= MAX_LABEL_TEXT {
            out.push('\u{2026}');
            break;
        }
        out.push(c);
    }
    out
}

/// Case-insensitive line matches inside files under `root`.
///
/// Stops early at the hit cap, the total-byte budget, the file-count cap, the
/// wall-clock budget or a set `cancel`. Hidden entries, symlinks, binary files
/// and the usual build/dependency directories are skipped. Directories are
/// visited depth-first with names sorted, so a given tree always scans the
/// same way.
pub(crate) fn scan_files(
    root: &Path,
    query: &str,
    limits: ScanLimits,
    cancel: &AtomicBool,
) -> Vec<FileHit> {
    let mut out = Vec::new();
    let needle = query.to_lowercase();
    if needle.is_empty() || limits.max_hits == 0 {
        return out;
    }
    let start = Instant::now();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let (mut files, mut bytes) = (0usize, 0u64);
    while let Some((dir, depth)) = stack.pop() {
        if stop(&out, start, &limits, cancel) {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        paths.sort();
        for path in paths {
            if stop(&out, start, &limits, cancel) {
                break;
            }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') {
                continue;
            }
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            let kind = meta.file_type();
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                if depth < limits.max_depth && !SKIP_DIRS.contains(&name) {
                    stack.push((path, depth + 1));
                }
                continue;
            }
            if !kind.is_file() || files >= limits.max_files || bytes >= limits.max_total_bytes {
                continue;
            }
            if meta.len() > limits.max_file_bytes {
                continue;
            }
            let Some(content) = read_capped(&path, limits.max_file_bytes) else {
                continue;
            };
            files += 1;
            bytes += content.len() as u64;
            if looks_binary(&content) {
                continue;
            }
            for (i, line) in String::from_utf8_lossy(&content).lines().enumerate() {
                if line.to_lowercase().contains(&needle) {
                    out.push(FileHit {
                        path: path.clone(),
                        line: i + 1,
                        text: preview(line),
                    });
                    if out.len() >= limits.max_hits {
                        return out;
                    }
                }
            }
        }
    }
    out
}

/// Whether any budget is spent.
fn stop(hits: &[FileHit], start: Instant, limits: &ScanLimits, cancel: &AtomicBool) -> bool {
    cancel.load(Ordering::Relaxed) || hits.len() >= limits.max_hits || start.elapsed() > limits.time
}

/// Read at most `max` bytes; a larger file is not read.
fn read_capped(path: &Path, max: u64) -> Option<Vec<u8>> {
    use std::io::Read;
    let file = std::fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    file.take(max + 1).read_to_end(&mut buf).ok()?;
    (buf.len() as u64 <= max).then_some(buf)
}

/// A NUL byte in the leading bytes marks a file too binary to show as text.
fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|b| *b == 0)
}

/// Case-insensitive matches inside one pane's scrollback, newest lines first.
///
/// `lines` pairs an absolute buffer line with its cells (see
/// `ATerm::line_chars_abs`); reusing `find_in_cells` keeps the reported
/// columns correct through wide characters.
pub(crate) fn scan_scrollback(
    lines: &[ScrollLine],
    query: &str,
    max_hits: usize,
) -> Vec<ScrollHit> {
    let mut out = Vec::new();
    if query.is_empty() || max_hits == 0 {
        return out;
    }
    for (line, cells) in lines {
        for (col, width) in super::find_in_cells(cells, query) {
            let text = cells.iter().map(|(_, c, _)| *c).collect::<String>();
            out.push(ScrollHit {
                line: *line,
                col,
                width,
                text: preview(&text),
            });
            if out.len() >= max_hits {
                return out;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mtty-quick-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn never() -> AtomicBool {
        AtomicBool::new(false)
    }

    #[test]
    fn file_scan_finds_lines_case_insensitively() {
        let dir = temp_dir("case");
        std::fs::write(dir.join("a.txt"), "Hello World\nsecond hello\n").unwrap();
        std::fs::write(dir.join("b.rs"), "nothing here\n").unwrap();
        let hits = scan_files(&dir, "HELLO", ScanLimits::default(), &never());
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert!(hits.iter().all(|h| h.path == dir.join("a.txt")));
        assert_eq!((hits[0].line, hits[0].text.as_str()), (1, "Hello World"));
        assert_eq!((hits[1].line, hits[1].text.as_str()), (2, "second hello"));
    }

    #[test]
    fn file_scan_is_bounded_and_skips_noise() {
        let dir = temp_dir("bounded");
        // Hidden, binary, oversized and build-tree files are all skipped.
        std::fs::write(dir.join(".hidden.txt"), "needle\n").unwrap();
        std::fs::write(dir.join("bin.dat"), b"nee\0dle\n").unwrap();
        std::fs::create_dir_all(dir.join("target")).unwrap();
        std::fs::write(dir.join("target/x.txt"), "needle\n").unwrap();
        std::fs::write(dir.join("big.txt"), "needle\n".repeat(50)).unwrap();
        std::fs::write(dir.join("ok.txt"), "one needle\ntwo needle\n").unwrap();

        let limits = ScanLimits {
            max_file_bytes: 64,
            max_hits: 1,
            ..ScanLimits::default()
        };
        let hits = scan_files(&dir, "needle", limits, &never());
        assert_eq!(hits.len(), 1, "hit cap not respected: {hits:?}");
        assert_eq!(hits[0].path, dir.join("ok.txt"));

        // A larger cap sees the two lines of ok.txt but never the noise.
        let limits = ScanLimits {
            max_hits: 10,
            ..limits
        };
        let hits = scan_files(&dir, "needle", limits, &never());
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert!(hits.iter().all(|h| h.path == dir.join("ok.txt")));
    }

    #[test]
    fn file_scan_stops_when_cancelled() {
        let dir = temp_dir("cancel");
        std::fs::write(dir.join("a.txt"), "needle\n").unwrap();
        let cancel = AtomicBool::new(true);
        assert!(scan_files(&dir, "needle", ScanLimits::default(), &cancel).is_empty());
    }

    #[test]
    fn scrollback_scan_columns_follow_wide_characters() {
        // "目录 abc 目录": 目(0,2) 录(2,2) ' '(4) a(5) b(6) c(7) ' '(8) 目(9,2) 录(11,2)
        let cells = vec![
            (0, '目', 2),
            (2, '录', 2),
            (4, ' ', 1),
            (5, 'a', 1),
            (6, 'b', 1),
            (7, 'c', 1),
            (8, ' ', 1),
            (9, '目', 2),
            (11, '录', 2),
        ];
        let lines = vec![(7usize, cells)];
        let hits = scan_scrollback(&lines, "ABC", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].line, hits[0].col, hits[0].width), (7, 5, 3));
        assert_eq!(hits[0].text, "目录 abc 目录");

        let hits = scan_scrollback(&lines, "目录", 10);
        assert_eq!(hits.len(), 2);
        assert_eq!((hits[0].col, hits[0].width), (0, 4));
        assert_eq!((hits[1].col, hits[1].width), (9, 4));
    }

    #[test]
    fn scrollback_scan_is_capped_and_ordered() {
        let a = vec![(0u16, 'x', 1u16), (1, 'y', 1)];
        let lines = vec![(2usize, a.clone()), (1, a.clone()), (0, a)];
        let hits = scan_scrollback(&lines, "xy", 2);
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert_eq!(hits[0].line, 2);
        assert_eq!(hits[1].line, 1);
    }

    #[test]
    fn preview_caps_long_lines() {
        let long = "a".repeat(MAX_LABEL_TEXT + 5);
        let p = preview(&long);
        assert_eq!(p.chars().count(), MAX_LABEL_TEXT + 1);
        assert!(p.ends_with('\u{2026}'));
    }
}
