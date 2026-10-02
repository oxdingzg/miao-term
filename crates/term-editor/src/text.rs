//! Decoding a file into a rope and back, keeping its BOM and line endings.

use ropey::Rope;

/// The line ending a document uses for new lines. Existing lines keep
/// whatever ending they have; only inserted line breaks use this one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    CrLf,
}

impl LineEnding {
    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
        }
    }

    /// The ending most lines in the first 64 KiB use (LF when there are none,
    /// or on a tie).
    pub fn detect(text: &str) -> LineEnding {
        let mut end = text.len().min(64 * 1024);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let head = &text[..end];
        let lf = head.matches('\n').count();
        let crlf = head.matches("\r\n").count();
        if crlf * 2 > lf {
            LineEnding::CrLf
        } else {
            LineEnding::Lf
        }
    }

    /// `text` with every line break (`\r\n` or `\n`) written as this ending.
    pub fn normalize(self, text: &str) -> String {
        let unified = text.replace("\r\n", "\n");
        match self {
            LineEnding::Lf => unified,
            LineEnding::CrLf => unified.replace('\n', "\r\n"),
        }
    }
}

/// Why a file cannot be opened as text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// Not valid UTF-8 (a binary file, or a legacy encoding); `valid_up_to` is
    /// the byte offset of the first bad sequence.
    NotUtf8 { valid_up_to: usize },
    /// Contains NUL bytes: almost certainly binary.
    Binary,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::NotUtf8 { valid_up_to } => {
                write!(f, "not UTF-8 text (invalid byte at offset {valid_up_to})")
            }
            DecodeError::Binary => write!(f, "binary file"),
        }
    }
}

impl std::error::Error for DecodeError {}

const BOM: &str = "\u{feff}";

/// A file's bytes as a rope, whether it had a BOM, and its line ending.
pub fn decode(bytes: &[u8]) -> Result<(Rope, bool, LineEnding), DecodeError> {
    let text = std::str::from_utf8(bytes).map_err(|e| DecodeError::NotUtf8 {
        valid_up_to: e.valid_up_to(),
    })?;
    if memchr_nul(text.as_bytes()) {
        return Err(DecodeError::Binary);
    }
    let (bom, text) = match text.strip_prefix(BOM) {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    Ok((Rope::from_str(text), bom, LineEnding::detect(text)))
}

/// The bytes to write back: the BOM if the file had one, then the text.
pub fn encode(rope: &Rope, bom: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(rope.len_bytes() + 3);
    if bom {
        out.extend_from_slice(BOM.as_bytes());
    }
    for chunk in rope.chunks() {
        out.extend_from_slice(chunk.as_bytes());
    }
    out
}

fn memchr_nul(bytes: &[u8]) -> bool {
    bytes.contains(&0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_bom_and_line_endings() {
        let bytes = "\u{feff}a\r\nb\r\n".as_bytes();
        let (rope, bom, ending) = decode(bytes).unwrap();
        assert!(bom);
        assert_eq!(ending, LineEnding::CrLf);
        assert_eq!(rope.to_string(), "a\r\nb\r\n");
        assert_eq!(encode(&rope, bom), bytes);
    }

    #[test]
    fn line_ending_follows_the_majority() {
        assert_eq!(LineEnding::detect("a\nb\nc\r\n"), LineEnding::Lf);
        assert_eq!(LineEnding::detect("a\r\nb\r\nc\n"), LineEnding::CrLf);
        assert_eq!(LineEnding::detect("no breaks"), LineEnding::Lf);
        assert_eq!(LineEnding::CrLf.normalize("a\nb\r\nc"), "a\r\nb\r\nc");
        assert_eq!(LineEnding::Lf.normalize("a\r\nb\nc"), "a\nb\nc");
    }

    #[test]
    fn rejects_binary_and_non_utf8() {
        assert_eq!(decode(b"ab\0cd"), Err(DecodeError::Binary));
        assert_eq!(
            decode(b"ok \xff\xfe"),
            Err(DecodeError::NotUtf8 { valid_up_to: 3 })
        );
    }
}
