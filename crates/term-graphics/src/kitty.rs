//! Kitty graphics protocol (APC `G`): parameter parsing and image decoding.

use crate::{decode_bitmap, Image, KittyCmd};

/// Parse the bytes after `ESC _ G` (i.e. `key=val,key=val;<payload>`).
pub fn parse(body: &[u8]) -> KittyCmd {
    let (params_b, data) = match body.iter().position(|&b| b == b';') {
        Some(i) => (&body[..i], body[i + 1..].to_vec()),
        None => (body, Vec::new()),
    };
    let params = String::from_utf8_lossy(params_b);
    let mut cmd = KittyCmd {
        action: 't',
        id: None,
        cols: None,
        rows: None,
        move_cursor: false,
        z: 0,
        x: 0,
        y: 0,
        format: 32,
        compressed: false,
        more: false,
        size: None,
        data,
    };
    for part in params.split(',') {
        let Some((k, v)) = part.split_once('=') else {
            continue;
        };
        match k {
            "a" => cmd.action = v.chars().next().unwrap_or('t'),
            "i" => cmd.id = v.parse().ok(),
            "c" => cmd.cols = v.parse().ok(),
            "r" => cmd.rows = v.parse().ok(),
            // `C=0` requests moving the cursor (default is not to).
            "C" => cmd.move_cursor = v == "0",
            "z" => cmd.z = v.parse().unwrap_or(0),
            "X" => cmd.x = v.parse().unwrap_or(0),
            "Y" => cmd.y = v.parse().unwrap_or(0),
            "f" => cmd.format = v.parse().unwrap_or(32),
            "o" => cmd.compressed = v == "z",
            "m" => cmd.more = v == "1",
            "s" => {
                if let Some((w, h)) = v.split_once('x') {
                    cmd.size = Some((w.parse().unwrap_or(0), h.parse().unwrap_or(0)));
                }
            }
            _ => {}
        }
    }
    cmd
}

/// Decode a display command into an image (`a=T` / `a=p`).
pub fn decode(cmd: &KittyCmd, max_pixels: usize) -> Option<Image> {
    use base64::Engine;
    let b64: Vec<u8> = cmd
        .data
        .iter()
        .copied()
        .filter(|b| !b.is_ascii_whitespace())
        .collect();
    let raw = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    if cmd.format == 100 {
        return decode_bitmap(&raw, 100, cmd.compressed, max_pixels);
    }
    let inflated = if cmd.compressed {
        use std::io::Read;
        let mut out = Vec::new();
        flate2::read::ZlibDecoder::new(&raw[..])
            .take((max_pixels * 4 + 64) as u64)
            .read_to_end(&mut out)
            .ok()?;
        out
    } else {
        raw
    };
    let channels = if cmd.format == 24 { 3 } else { 4 };
    let (w, h) = cmd.size.unwrap_or_else(|| {
        let n = inflated.len() / channels;
        (n as u32, 1)
    });
    if w == 0 || h == 0 || (w as usize * h as usize) > max_pixels {
        return None;
    }
    if inflated.len() < w as usize * h as usize * channels {
        return None;
    }
    let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
    for px in inflated
        .chunks_exact(channels)
        .take(w as usize * h as usize)
    {
        rgba.extend_from_slice(&px[..3]);
        rgba.push(if channels == 4 { px[3] } else { 255 });
    }
    Some(Image {
        width: w,
        height: h,
        rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_parameters() {
        let cmd = parse(b"a=T,f=100,c=10,r=5,i=7,z=1,X=3,Y=4,m=1;somedata");
        assert_eq!(cmd.action, 'T');
        assert_eq!(cmd.format, 100);
        assert_eq!(cmd.cols, Some(10));
        assert_eq!(cmd.rows, Some(5));
        assert_eq!(cmd.id, Some(7));
        assert_eq!(cmd.z, 1);
        assert_eq!((cmd.x, cmd.y), (3, 4));
        assert!(cmd.more);
        assert_eq!(cmd.data, b"somedata");
    }

    #[test]
    fn decodes_raw_rgb_with_size() {
        // 2x1 RGB: base64 of bytes [0,0,0, 255,0,0]
        let bytes: [u8; 6] = [0, 0, 0, 255, 0, 0];
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
        let cmd = parse(format!("a=T,f=24,s=2x1;{b64}").as_bytes());
        let img = decode(&cmd, 100).unwrap();
        assert_eq!((img.width, img.height), (2, 1));
        assert_eq!(img.rgba, vec![0, 0, 0, 255, 255, 0, 0, 255]);
    }

    #[test]
    fn rejects_bad_base64() {
        let cmd = parse(b"a=T,f=24;!!!!");
        assert!(decode(&cmd, 100).is_none());
    }
}
