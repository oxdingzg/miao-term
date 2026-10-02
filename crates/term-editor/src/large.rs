//! Large files viewed without loading them (ADR 0034, large-file mode).
//!
//! A [`LargeFile`] reads the file on demand: a background thread scans it
//! once for line breaks and keeps a sparse index (the start of every
//! [`STRIDE`]th line), so a screen of lines anywhere in a gigabyte file is a
//! short read from the nearest mark, and memory stays small whatever the
//! file's size. Search scans the file in chunks on the caller's thread (run
//! it on a background thread).

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

/// Lines between index marks.
pub const STRIDE: usize = 1024;

/// A line longer than this shows its start and an ellipsis; the rest is
/// skipped (a minified file can be one line of a gigabyte).
pub const MAX_LINE_BYTES: usize = 64 << 10;

/// Bytes read at a time while scanning.
const CHUNK: usize = 4 << 20;

#[derive(Default)]
struct Index {
    /// `marks[k]`: byte offset where line `k * STRIDE` starts.
    marks: Vec<u64>,
    /// Line breaks seen so far.
    breaks: usize,
    /// Bytes scanned so far.
    scanned: u64,
}

/// A file read in place, with a line index built in the background.
pub struct LargeFile {
    path: PathBuf,
    file: File,
    len: u64,
    index: Mutex<Index>,
    done: AtomicBool,
}

fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    #[cfg(unix)]
    {
        std::os::unix::fs::FileExt::read_at(file, buf, offset)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::FileExt::seek_read(file, buf, offset)
    }
}

impl LargeFile {
    /// Open `path` and start indexing it on a background thread, which
    /// calls the background waker as it makes progress and stops if the
    /// file is dropped.
    pub fn open(path: &Path) -> io::Result<Arc<LargeFile>> {
        let file = File::open(path)?;
        let len = file.metadata()?.len();
        let large = Arc::new(LargeFile {
            path: path.to_path_buf(),
            file,
            len,
            index: Mutex::new(Index {
                marks: vec![0],
                ..Index::default()
            }),
            done: AtomicBool::new(false),
        });
        let weak = Arc::downgrade(&large);
        std::thread::Builder::new()
            .name("mtty-index".into())
            .spawn(move || index(weak))?;
        Ok(large)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn len_bytes(&self) -> u64 {
        self.len
    }

    /// The line index is complete.
    pub fn indexed(&self) -> bool {
        self.done.load(Ordering::Acquire)
    }

    /// Share of the file indexed, 0.0–1.0.
    pub fn progress(&self) -> f32 {
        if self.len == 0 {
            return 1.0;
        }
        let scanned = self.index.lock().unwrap_or_else(|e| e.into_inner()).scanned;
        (scanned as f64 / self.len as f64) as f32
    }

    /// Lines known so far: every line once indexing is done (line breaks
    /// plus one, as an editor counts them).
    pub fn line_count(&self) -> usize {
        self.index.lock().unwrap_or_else(|e| e.into_inner()).breaks + 1
    }

    /// Up to `max_lines` lines from line `first`, at most about `max_bytes`
    /// in all, joined with `\n` (CR before LF dropped, invalid UTF-8
    /// replaced, lines over [`MAX_LINE_BYTES`] cut with `…`). Returns the
    /// text and how many lines it holds; past the indexed part it returns
    /// what it can.
    pub fn read_lines(&self, first: usize, max_lines: usize, max_bytes: usize) -> (String, usize) {
        let mark = {
            let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
            let k = (first / STRIDE).min(index.marks.len() - 1);
            (k, index.marks[k])
        };
        let mut skip = first - mark.0 * STRIDE;
        let mut out: Vec<u8> = Vec::new();
        let mut line: Vec<u8> = Vec::new();
        let mut cut = false;
        let mut lines = 0;
        let mut offset = mark.1;
        let mut buf = vec![0u8; CHUNK.min(1 << 20)];
        let finish = |line: &mut Vec<u8>, cut: &mut bool, out: &mut Vec<u8>| {
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            if !out.is_empty() {
                out.push(b'\n');
            }
            out.extend_from_slice(line);
            if *cut {
                out.extend_from_slice("\u{2026}".as_bytes());
            }
            line.clear();
            *cut = false;
        };
        'read: while offset < self.len && lines < max_lines {
            let n = match read_at(&self.file, &mut buf, offset) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let mut at = 0;
            while at < n {
                let rest = &buf[at..n];
                let brk = memchr::memchr(b'\n', rest);
                let piece = &rest[..brk.unwrap_or(rest.len())];
                if skip == 0 {
                    let room = MAX_LINE_BYTES.saturating_sub(line.len());
                    line.extend_from_slice(&piece[..piece.len().min(room)]);
                    cut |= piece.len() > room;
                }
                match brk {
                    Some(i) => {
                        at += i + 1;
                        if skip > 0 {
                            skip -= 1;
                            continue;
                        }
                        finish(&mut line, &mut cut, &mut out);
                        lines += 1;
                        if lines >= max_lines || out.len() >= max_bytes {
                            break 'read;
                        }
                    }
                    None => at = n,
                }
            }
            offset += n as u64;
        }
        // The file's last line: what follows the final break, possibly
        // empty (as an editor shows a file ending with a newline).
        if skip == 0 && lines < max_lines && offset >= self.len {
            let first_line = out.is_empty() && lines == 0;
            if !first_line {
                out.push(b'\n');
            }
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            out.extend_from_slice(&line);
            if cut {
                out.extend_from_slice("\u{2026}".as_bytes());
            }
            lines += 1;
        }
        (String::from_utf8_lossy(&out).into_owned(), lines)
    }

    /// The line holding byte `offset`.
    pub fn line_of(&self, offset: u64) -> usize {
        let (k, start) = {
            let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
            let k = index.marks.partition_point(|&m| m <= offset).max(1) - 1;
            (k, index.marks[k])
        };
        k * STRIDE + self.count_breaks(start, offset.min(self.len))
    }

    /// The byte offset where `line` starts (scanning from the nearest mark).
    pub fn line_start(&self, line: usize) -> u64 {
        let (k, start) = {
            let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
            let k = (line / STRIDE).min(index.marks.len() - 1);
            (k, index.marks[k])
        };
        let mut need = line - k * STRIDE;
        let mut offset = start;
        let mut buf = vec![0u8; 1 << 20];
        while need > 0 && offset < self.len {
            let n = match read_at(&self.file, &mut buf, offset) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            for i in memchr::memchr_iter(b'\n', &buf[..n]) {
                need -= 1;
                if need == 0 {
                    return offset + i as u64 + 1;
                }
            }
            offset += n as u64;
        }
        offset.min(self.len)
    }

    fn count_breaks(&self, from: u64, to: u64) -> usize {
        let mut count = 0;
        let mut offset = from;
        let mut buf = vec![0u8; 1 << 20];
        while offset < to {
            let want = ((to - offset) as usize).min(buf.len());
            let n = match read_at(&self.file, &mut buf[..want], offset) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            count += memchr::memchr_iter(b'\n', &buf[..n]).count();
            offset += n as u64;
        }
        count
    }

    /// Every match of `query` (literal, ignoring case) as byte ranges, in
    /// order, reported through `found` as it goes; stops early when `cancel`
    /// is set or after `cap` matches. Returns whether it reached the end.
    pub fn search(
        &self,
        query: &str,
        cap: usize,
        cancel: &AtomicBool,
        mut found: impl FnMut(u64, u64),
    ) -> bool {
        if query.is_empty() {
            return true;
        }
        let Ok(re) = regex::bytes::RegexBuilder::new(&regex::escape(query))
            .case_insensitive(true)
            .build()
        else {
            return true;
        };
        // Chunks overlap so a match across a boundary is seen whole: a chunk
        // reports the matches that start before its overlap, and the next
        // chunk starts there.
        let overlap = query.len() * 4 + 16;
        let mut buf = vec![0u8; CHUNK + overlap];
        let mut count = 0;
        let mut start: u64 = 0;
        while start < self.len {
            if cancel.load(Ordering::Relaxed) {
                return false;
            }
            let n = match read_at(&self.file, &mut buf, start) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let end = start + n as u64;
            let last = end >= self.len || n <= overlap;
            let next = if last { u64::MAX } else { end - overlap as u64 };
            for m in re.find_iter(&buf[..n]) {
                let (a, b) = (start + m.start() as u64, start + m.end() as u64);
                if a >= next {
                    break;
                }
                found(a, b);
                count += 1;
                if count >= cap {
                    return false;
                }
            }
            if last {
                break;
            }
            start = next;
        }
        true
    }
}

/// Scan the file for line breaks, publishing marks as they are found.
fn index(weak: Weak<LargeFile>) {
    let mut buf = vec![0u8; CHUNK];
    let mut offset = 0u64;
    let mut breaks = 0usize;
    loop {
        let Some(file) = weak.upgrade() else {
            return;
        };
        let n = match read_at(&file.file, &mut buf, offset) {
            Ok(0) | Err(_) => 0,
            Ok(n) => n,
        };
        let mut marks = Vec::new();
        for i in memchr::memchr_iter(b'\n', &buf[..n]) {
            breaks += 1;
            if breaks % STRIDE == 0 {
                marks.push(offset + i as u64 + 1);
            }
        }
        offset += n as u64;
        {
            let mut index = file.index.lock().unwrap_or_else(|e| e.into_inner());
            index.marks.extend(marks);
            index.breaks = breaks;
            index.scanned = offset;
        }
        let finished = n == 0 || offset >= file.len;
        if finished {
            file.done.store(true, Ordering::Release);
        }
        crate::syntax::wake_background();
        if finished {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str, bytes: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mtty-large-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn indexed(path: &Path) -> Arc<LargeFile> {
        let file = LargeFile::open(path).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !file.indexed() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        file
    }

    fn numbered(lines: usize) -> String {
        (0..lines).map(|i| format!("line {i}\n")).collect()
    }

    #[test]
    fn lines_anywhere_come_from_the_nearest_mark() {
        let text = numbered(STRIDE * 5 + 7);
        let file = indexed(&temp("numbered.txt", text.as_bytes()));
        assert_eq!(file.line_count(), STRIDE * 5 + 8, "breaks plus one");
        assert_eq!(file.progress(), 1.0);
        let (s, n) = file.read_lines(0, 2, usize::MAX);
        assert_eq!((s.as_str(), n), ("line 0\nline 1", 2));
        let at = STRIDE * 3 + 5;
        let (s, n) = file.read_lines(at, 3, usize::MAX);
        assert_eq!(n, 3);
        assert_eq!(s, format!("line {at}\nline {}\nline {}", at + 1, at + 2));
        // The end: the last line, then the empty line after the final break.
        let last = STRIDE * 5 + 6;
        let (s, n) = file.read_lines(last, 10, usize::MAX);
        assert_eq!((s, n), (format!("line {last}\n"), 2));
        assert_eq!(
            file.line_start(at),
            text.find(&format!("line {at}\n")).unwrap() as u64
        );
        let offset = text.find(&format!("line {}\n", STRIDE * 4 + 2)).unwrap() as u64 + 3;
        assert_eq!(file.line_of(offset), STRIDE * 4 + 2);
        assert_eq!(file.line_of(0), 0);
    }

    #[test]
    fn crlf_long_lines_and_bad_utf8_are_shown_safely() {
        let mut bytes = b"a\r\nb\r\n".to_vec();
        bytes.extend(std::iter::repeat_n(b'x', MAX_LINE_BYTES + 10));
        bytes.extend_from_slice(b"\nok \xff end");
        let file = indexed(&temp("odd.txt", &bytes));
        let (s, n) = file.read_lines(0, 10, usize::MAX);
        assert_eq!(n, 4, "{s:?}");
        let lines: Vec<&str> = s.split('\n').collect();
        assert_eq!(&lines[..2], &["a", "b"]);
        assert_eq!(lines[2].len(), MAX_LINE_BYTES + "\u{2026}".len());
        assert!(lines[2].ends_with('\u{2026}'));
        assert_eq!(lines[3], "ok \u{fffd} end");
    }

    #[test]
    fn search_finds_matches_across_chunk_boundaries_once() {
        // Matches just before, across and just after the first chunk's end
        // (its overlap region), in a file of several chunks.
        let mut bytes = vec![b'.'; 3 * CHUNK];
        let at = [CHUNK - 30, CHUNK - 3, CHUNK + 5, CHUNK + 30, 2 * CHUNK - 2];
        for &i in &at {
            bytes[i..i + 6].copy_from_slice(b"NeedLE");
        }
        bytes.extend_from_slice(b"\nneedle needle\n");
        let file = indexed(&temp("search.txt", &bytes));
        let mut hits = Vec::new();
        let cancel = AtomicBool::new(false);
        assert!(file.search("needle", 100, &cancel, |a, b| hits.push((a, b))));
        let starts: Vec<u64> = hits.iter().map(|h| h.0).collect();
        let mut want: Vec<u64> = at.iter().map(|&i| i as u64).collect();
        let tail = (3 * CHUNK + 1) as u64;
        want.extend([tail, tail + 7]);
        assert_eq!(starts, want, "each match once, in order");
        assert!(hits.iter().all(|&(a, b)| b == a + 6));
        let mut capped = 0;
        assert!(!file.search("needle", 2, &cancel, |_, _| capped += 1));
        assert_eq!(capped, 2);
    }

    #[test]
    fn empty_and_unterminated_files() {
        let empty = indexed(&temp("empty.txt", b""));
        assert_eq!(empty.line_count(), 1);
        assert_eq!(empty.read_lines(0, 5, usize::MAX), (String::new(), 1));
        let one = indexed(&temp("one.txt", b"only"));
        assert_eq!(one.read_lines(0, 5, usize::MAX), ("only".to_string(), 1));
        let ended = indexed(&temp("ended.txt", b"a\n"));
        assert_eq!(ended.line_count(), 2);
        assert_eq!(ended.read_lines(0, 5, usize::MAX), ("a\n".to_string(), 2));
    }
}
