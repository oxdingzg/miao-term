//! Kitty graphics protocol (APC `G`): parameter parsing and image decoding.

use crate::{decode_bitmap, Image, KittyCmd};

/// Parse the bytes after `ESC _ G` (i.e. `key=val,key=val;<payload>`).
///
/// Parsed as a key/value map first: a few keys mean different things depending
/// on the action (`s` is size for transmit, animation state for `a=a`; `c` is
/// columns for transmit, loop count for `a=a`).
pub fn parse(body: &[u8]) -> KittyCmd {
    let (params_b, data) = match body.iter().position(|&b| b == b';') {
        Some(i) => (&body[..i], body[i + 1..].to_vec()),
        None => (body, Vec::new()),
    };
    let params = String::from_utf8_lossy(params_b);
    let kv: Vec<(&str, &str)> = params
        .split(',')
        .filter_map(|p| p.split_once('='))
        .collect();
    let get = |k: &str| kv.iter().find(|(k2, _)| *k2 == k).map(|(_, v)| *v);
    let getu = |k: &str| get(k).and_then(|v| v.parse::<u32>().ok());

    let action = get("a").and_then(|s| s.chars().next()).unwrap_or('t');
    let is_anim = action == 'a';
    let size = get("s")
        .filter(|v| v.contains('x'))
        .and_then(|v| v.split_once('x'))
        .map(|(w, h)| (w.parse().unwrap_or(0), h.parse().unwrap_or(0)));

    KittyCmd {
        action,
        id: getu("i").map(u64::from),
        cols: if is_anim {
            None
        } else {
            getu("c").map(|n| n as u16)
        },
        rows: getu("r").map(|n| n as u16),
        // `C=0` requests moving the cursor (default is not to).
        move_cursor: get("C") == Some("0"),
        z: get("z").and_then(|v| v.parse().ok()).unwrap_or(0),
        x: get("X").and_then(|v| v.parse().ok()).unwrap_or(0),
        y: get("Y").and_then(|v| v.parse().ok()).unwrap_or(0),
        format: getu("f").unwrap_or(32),
        compressed: get("o") == Some("z"),
        more: get("m") == Some("1"),
        size,
        delete: get("d").and_then(|v| v.chars().next()),
        cell_x: getu("x").map(|n| n as u16),
        cell_y: getu("y").map(|n| n as u16),
        state: if is_anim { getu("s") } else { None },
        loops: if is_anim { getu("c") } else { None },
        data,
    }
}

/// Decode a display command into an image (`a=T` / `a=p`), or a frame (`a=f`).
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
        assert_eq!(cmd.delete, None);
        let del = parse(b"a=d,d=p,x=4,y=2;i=1");
        assert_eq!(del.delete, Some('p'));
        assert_eq!((del.cell_x, del.cell_y), (Some(4), Some(2)));
    }

    #[test]
    fn animation_params_differ_by_action() {
        // Transmit: `s` is the pixel size, `c` the columns.
        let t = parse(b"a=T,f=24,s=2x1,c=3;i=1;AAAA");
        assert_eq!(t.size, Some((2, 1)));
        assert_eq!(t.cols, Some(3));
        assert_eq!(t.state, None);
        // Animation control: `s` is the state, `c` the loop count.
        let a = parse(b"a=a,i=1,s=1,c=5");
        assert_eq!(a.state, Some(1));
        assert_eq!(a.loops, Some(5));
        assert_eq!(a.size, None);
    }

    #[test]
    fn decodes_raw_rgb_with_size() {
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
