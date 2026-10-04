//! The output ring (ADR 0041 §2): recent output as whole frames, addressed by
//! absolute byte offsets since the program started.

use std::collections::VecDeque;

use crate::scanner::Modes;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub offset: u64,
    pub boundary: bool,
    pub bytes: Vec<u8>,
    /// The private modes in force where this frame starts.
    pub modes_before: Modes,
}

impl Frame {
    pub fn end(&self) -> u64 {
        self.offset + self.bytes.len() as u64
    }
}

/// What a client attaching at some offset receives.
#[derive(Debug, PartialEq, Eq)]
pub struct Replay<'a> {
    /// `Some(oldest frame start, its modes)` when the ring no longer reaches
    /// the requested offset.
    pub truncated: Option<(u64, &'a Modes)>,
    /// Frames from the requested offset on; the first may start before it
    /// when the offset falls inside a frame, in which case it is cut.
    pub frames: Vec<Frame>,
}

pub struct Ring {
    frames: VecDeque<Frame>,
    bytes: usize,
    cap: usize,
    /// Where the next frame starts.
    end: u64,
    /// Modes at `end` when the ring is empty (all evicted or never filled).
    modes_at_end: Modes,
}

impl Ring {
    pub fn new(cap: usize) -> Self {
        Self {
            frames: VecDeque::new(),
            bytes: 0,
            cap: cap.max(1),
            end: 0,
            modes_at_end: Modes::default(),
        }
    }

    pub fn end(&self) -> u64 {
        self.end
    }

    /// Append a frame; evicts whole frames from the front beyond the cap
    /// (always keeping the newest).
    pub fn push(
        &mut self,
        boundary: bool,
        bytes: Vec<u8>,
        modes_before: Modes,
        modes_after: Modes,
    ) {
        let frame = Frame {
            offset: self.end,
            boundary,
            bytes,
            modes_before,
        };
        self.end = frame.end();
        self.bytes += frame.bytes.len();
        self.frames.push_back(frame);
        self.modes_at_end = modes_after;
        while self.bytes > self.cap && self.frames.len() > 1 {
            let old = self.frames.pop_front().expect("more than one frame");
            self.bytes -= old.bytes.len();
        }
    }

    /// The output from `from` on.
    pub fn since(&self, from: u64) -> Replay<'_> {
        let from = from.min(self.end);
        let oldest = self.frames.front().map_or(self.end, |f| f.offset);
        if from < oldest {
            let modes = self
                .frames
                .front()
                .map_or(&self.modes_at_end, |f| &f.modes_before);
            return Replay {
                truncated: Some((oldest, modes)),
                frames: self.frames.iter().cloned().collect(),
            };
        }
        let frames = self
            .frames
            .iter()
            .filter(|f| f.end() > from)
            .map(|f| {
                if f.offset >= from {
                    return f.clone();
                }
                let cut = (from - f.offset) as usize;
                Frame {
                    offset: from,
                    bytes: f.bytes[cut..].to_vec(),
                    ..f.clone()
                }
            })
            .collect();
        Replay {
            truncated: None,
            frames,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring(cap: usize, chunks: &[&[u8]]) -> Ring {
        let mut r = Ring::new(cap);
        for c in chunks {
            r.push(true, c.to_vec(), Modes::default(), Modes::default());
        }
        r
    }

    fn text(replay: &Replay) -> Vec<u8> {
        replay.frames.iter().flat_map(|f| f.bytes.clone()).collect()
    }

    #[test]
    fn replays_from_an_offset() {
        let r = ring(100, &[b"abc", b"de", b"fgh"]);
        assert_eq!(r.end(), 8);
        assert_eq!(text(&r.since(0)), b"abcdefgh");
        assert_eq!(text(&r.since(5)), b"fgh");
        assert_eq!(r.since(5).frames[0].offset, 5);
        assert_eq!(text(&r.since(4)), b"efgh", "inside a frame: cut");
        assert_eq!(r.since(4).frames[0].offset, 4);
        assert!(r.since(8).frames.is_empty());
        assert!(r.since(99).frames.is_empty(), "past the end");
        assert_eq!(r.since(0).truncated, None);
    }

    #[test]
    fn evicts_whole_frames_and_reports_truncation() {
        let mut r = Ring::new(5);
        let mut alt = Modes::default();
        let mut scanner = crate::scanner::Scanner::new();
        scanner.feed(b"\x1b[?1049h");
        alt.clone_from(scanner.modes());
        r.push(true, b"abc".to_vec(), Modes::default(), alt.clone());
        r.push(true, b"de".to_vec(), alt.clone(), alt.clone());
        r.push(true, b"fgh".to_vec(), alt.clone(), alt.clone());
        let replay = r.since(0);
        let (oldest, modes) = replay.truncated.unwrap();
        assert_eq!(oldest, 3);
        assert!(modes.alt_screen(), "the modes where the ring now starts");
        assert_eq!(text(&replay), b"defgh");
        assert_eq!(r.since(3).truncated, None);
        // The newest frame stays even when it alone exceeds the cap.
        let big = ring(2, &[b"abcdef"]);
        assert_eq!(text(&big.since(0)), b"abcdef");
    }
}
