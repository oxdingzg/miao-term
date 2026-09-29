//! Inline terminal graphics: a stateful scanner that pulls Sixel (DCS), Kitty
//! (APC `G`) and iTerm2 (`OSC 1337;File=`) payloads out of a raw PTY byte
//! stream, plus decoders for each. The scanner is protocol-agnostic about the
//! underlying VT parser: it only isolates the sequences so the caller can feed
//! ordinary text to the parser and render the images itself.
//!
//! Everything is bounded: an unterminated sequence is flushed back as text once
//! it exceeds [`MAX_SEQUENCE`], and decoders cap the pixel count.

pub mod kitty;
pub mod sixel;

/// Largest graphics sequence held while waiting for its terminator.
pub const MAX_SEQUENCE: usize = 32 * 1024 * 1024;

/// A decoded, straight (non-premultiplied) RGBA image.
#[derive(Clone, Debug)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    pub fn pixel_count(&self) -> usize {
        self.width as usize * self.height as usize
    }
}

/// A single Kitty APC command.
#[derive(Clone, Debug, PartialEq)]
pub struct KittyCmd {
    /// Action: `t`/`T` transfer, `p` put, `d` delete, `q` query.
    pub action: char,
    pub id: Option<u64>,
    pub cols: Option<u16>,
    pub rows: Option<u16>,
    /// `C=0` means "move the cursor" (default is not to move it).
    pub move_cursor: bool,
    pub z: i32,
    /// `X`/`Y`: pixel offsets from the cursor cell.
    pub x: i32,
    pub y: i32,
    /// Format: 24 = RGB, 32 = RGBA, 100 = PNG.
    pub format: u32,
    /// `o=z`: payload is zlib-compressed.
    pub compressed: bool,
    /// `m=1`: more chunks follow.
    pub more: bool,
    /// `s=WxH` pixel size (needed for raw formats).
    pub size: Option<(u32, u32)>,
    /// `d` (delete what): a = all, i = by id, p = at cell, c/r = column/row, z = z-index.
    pub delete: Option<char>,
    /// `x`/`y`: cell coordinates (viewport-relative) for `a=d,d=p/c/r`.
    pub cell_x: Option<u16>,
    pub cell_y: Option<u16>,
    /// The `;`-separated payload (raw bytes; usually base64 text).
    pub data: Vec<u8>,
}

/// A graphics command extracted from the stream.
#[derive(Clone, Debug, PartialEq)]
pub enum Graphic {
    Sixel { transparent: bool, data: Vec<u8> },
    Kitty(KittyCmd),
    Iterm2 { name: Option<String>, data: Vec<u8> },
}

/// Scanner output, in stream order.
#[derive(Clone, Debug, PartialEq)]
pub enum Segment {
    /// Ordinary bytes; feed these to the VT parser.
    Text(Vec<u8>),
    /// An extracted graphics command (do **not** feed to the VT parser).
    Graphics(Graphic),
}

impl Segment {
    pub fn text(&self) -> Option<&[u8]> {
        match self {
            Segment::Text(b) => Some(b),
            _ => None,
        }
    }
}

/// Streaming scanner for graphics sequences.
#[derive(Default)]
pub struct Scanner {
    buf: Vec<u8>,
}

impl Scanner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a chunk of PTY output; returns the segments it completed.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Segment> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        let mut i = 0usize;
        while i < self.buf.len() {
            // Find the next introducer: ESC P (DCS), ESC _ (APC), ESC ] (OSC).
            let intro = find_intro(&self.buf[i..]).map(|k| i + k);
            let Some(start) = intro else {
                break;
            };
            let esc = self.buf[start];
            let tag = self.buf[start + 1];
            let is_osc = tag == b']';
            let body_start = start + 2;
            let Some((end, term_len)) = find_terminator(&self.buf, body_start, is_osc) else {
                // Incomplete: hold from `start` (bounded).
                if start > i {
                    out.push(Segment::Text(self.buf[i..start].to_vec()));
                }
                if self.buf.len() - start > MAX_SEQUENCE {
                    // Malicious/unterminated: give it back as text.
                    out.push(Segment::Text(self.buf[start..].to_vec()));
                    self.buf.clear();
                    return out;
                }
                self.buf.drain(..start);
                return out;
            };
            let payload = &self.buf[body_start..end];
            match classify(payload) {
                Some(g) => {
                    if start > i {
                        out.push(Segment::Text(self.buf[i..start].to_vec()));
                    }
                    out.push(Segment::Graphics(g));
                }
                None => {
                    // Unknown sequence: keep as text so other features work.
                    out.push(Segment::Text(self.buf[i..end + term_len].to_vec()));
                }
            }
            i = end + term_len;
            let _ = esc;
        }
        if i < self.buf.len() {
            out.push(Segment::Text(self.buf[i..].to_vec()));
        }
        self.buf.clear();
        out
    }
}

/// Offset of the next `ESC P` / `ESC _` / `ESC ]` in `s`.
fn find_intro(s: &[u8]) -> Option<usize> {
    let mut k = 0;
    while k + 1 < s.len() {
        if s[k] == 0x1b {
            match s[k + 1] {
                b'P' | b'_' | b']' => return Some(k),
                _ => {}
            }
        }
        k += 1;
    }
    None
}

/// Find the end of a sequence body starting at `from`. Returns `(end, term_len)`
/// where `end` is exclusive of the terminator.
fn find_terminator(s: &[u8], from: usize, is_osc: bool) -> Option<(usize, usize)> {
    let mut k = from;
    while k < s.len() {
        match s[k] {
            0x1b if k + 1 < s.len() && s[k + 1] == b'\\' => return Some((k, 2)),
            0x9c => return Some((k, 1)),
            0x07 if is_osc => return Some((k, 1)),
            _ => {}
        }
        k += 1;
    }
    None
}

/// Classify a sequence body into a supported graphic, or `None` to pass through.
fn classify(payload: &[u8]) -> Option<Graphic> {
    match payload.first()? {
        // DCS: `...q<sixel data>`
        b'q' => Some(Graphic::Sixel {
            transparent: false,
            data: payload[1..].to_vec(),
        }),
        // APC Kitty: `G<params>;<data>`
        b'G' => Some(Graphic::Kitty(kitty::parse(&payload[1..]))),
        // OSC iTerm2 (and possibly other OSC we ignore).
        b'1' => {
            let body = payload.strip_prefix(b"1337;")?;
            iterm2(body)
        }
        _ => None,
    }
}

/// Parse `File=<params>:<base64>` from an OSC 1337 body.
fn iterm2(body: &[u8]) -> Option<Graphic> {
    let rest = body.strip_prefix(b"File=")?;
    let (params, data) = split_once(rest, b':')?;
    let params = String::from_utf8_lossy(params).to_string();
    let name = params
        .split(';')
        .find_map(|p| p.strip_prefix("name=").map(|s| s.to_string()));
    Some(Graphic::Iterm2 {
        name,
        data: data.to_vec(),
    })
}

fn split_once(s: &[u8], sep: u8) -> Option<(&[u8], &[u8])> {
    let i = s.iter().position(|&b| b == sep)?;
    Some((&s[..i], &s[i + 1..]))
}

/// Decode an iTerm2 base64 payload (raw PNG/JPEG/etc.).
pub fn decode_iterm2(data: &[u8], max_pixels: usize) -> Option<Image> {
    use base64::Engine;
    let text: Vec<u8> = data
        .iter()
        .copied()
        .filter(|b| !b.is_ascii_whitespace())
        .collect();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(text)
        .ok()?;
    decode_png(&bytes, max_pixels)
}

/// Decode a PNG (format 100) or raw RGB/RGBA (formats 24/32) with optional
/// zlib. `max_pixels` bounds the decoded size.
pub fn decode_bitmap(
    data: &[u8],
    format: u32,
    compressed: bool,
    max_pixels: usize,
) -> Option<Image> {
    let raw = if compressed {
        use std::io::Read;
        let mut out = Vec::new();
        flate2::read::ZlibDecoder::new(data)
            .take((max_pixels * 4 + 64) as u64)
            .read_to_end(&mut out)
            .ok()?;
        out
    } else {
        data.to_vec()
    };
    match format {
        100 => decode_png(&raw, max_pixels),
        24 | 32 => {
            let channels = if format == 24 { 3 } else { 4 };
            if raw.is_empty() || raw.len() % channels != 0 {
                return None;
            }
            rgba_from_raw(&raw, channels, max_pixels)
        }
        _ => None,
    }
}

/// Decode raw RGB/RGBA rows of 1 px height (raw formats carry no geometry, so
/// the caller derives width from `cols`; we infer height from length).
pub fn rgba_from_raw(raw: &[u8], channels: usize, max_pixels: usize) -> Option<Image> {
    let pixels = raw.len() / channels;
    if pixels == 0 || pixels > max_pixels {
        return None;
    }
    let mut rgba = Vec::with_capacity(pixels * 4);
    for px in raw.chunks_exact(channels) {
        rgba.extend_from_slice(&px[..3]);
        rgba.push(if channels == 4 { px[3] } else { 255 });
    }
    Some(Image {
        width: pixels as u32,
        height: 1,
        rgba,
    })
}

fn decode_png(bytes: &[u8], max_pixels: usize) -> Option<Image> {
    let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png).ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    if w as usize * h as usize > max_pixels {
        return None;
    }
    Some(Image {
        width: w,
        height: h,
        rgba: rgba.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_plain_text_through() {
        let mut s = Scanner::new();
        let segs = s.feed(b"hello world");
        assert_eq!(segs, vec![Segment::Text(b"hello world".to_vec())]);
    }

    #[test]
    fn extracts_a_sixel_sequence() {
        let mut s = Scanner::new();
        let segs = s.feed(b"before\x1bPq#0;2;0;0;0~-\x1b\\after");
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0].text(), Some(&b"before"[..]));
        assert!(matches!(segs[1], Segment::Graphics(Graphic::Sixel { .. })));
        assert_eq!(segs[2].text(), Some(&b"after"[..]));
    }

    #[test]
    fn holds_a_split_sequence_until_complete() {
        let mut s = Scanner::new();
        assert_eq!(s.feed(b"a\x1bPq#0"), vec![Segment::Text(b"a".to_vec())]);
        let segs = s.feed(b"~-\x1b\\b");
        assert_eq!(segs.len(), 2);
        assert!(matches!(segs[0], Segment::Graphics(Graphic::Sixel { .. })));
        assert_eq!(segs[1].text(), Some(&b"b"[..]));
    }

    #[test]
    fn extracts_kitty_and_iterm2() {
        let mut s = Scanner::new();
        let segs = s.feed(b"\x1b_Ga=T,f=24;AAAA\x1b\\");
        match &segs[0] {
            Segment::Graphics(Graphic::Kitty(c)) => assert_eq!(c.action, 'T'),
            other => panic!("{other:?}"),
        }
        let segs = s.feed(b"\x1b]1337;File=name=YWJj:AAAA\x07");
        assert!(matches!(
            &segs[0],
            Segment::Graphics(Graphic::Iterm2 { name: Some(n), .. }) if n == "YWJj"
        ));
    }

    #[test]
    fn unknown_dcs_passes_through() {
        let mut s = Scanner::new();
        let segs = s.feed(b"\x1bP+q544e\x1b\\");
        assert_eq!(segs, vec![Segment::Text(b"\x1bP+q544e\x1b\\".to_vec())]);
    }

    #[test]
    fn raw_rgba_decodes() {
        let img = decode_bitmap(&[1, 2, 3, 4, 5, 6], 24, false, 100).unwrap();
        assert_eq!((img.width, img.height), (2, 1));
        assert_eq!(img.rgba, vec![1, 2, 3, 255, 4, 5, 6, 255]);
    }
}
