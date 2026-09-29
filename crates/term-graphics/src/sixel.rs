//! A Sixel (DEC) decoder: RLE, 6-pixel bands, palette definitions, `$`/`-`.

use crate::Image;

/// Decode a Sixel payload (the bytes after the `q` introducer). `transparent`
/// comes from the DCS `P2` parameter (background stays transparent).
pub fn decode(data: &[u8], transparent: bool, max_pixels: usize) -> Option<Image> {
    let mut palette = default_palette();
    let mut cur = 0u8;
    let mut w = 0usize;
    let mut h = 0usize;
    let mut buf: Vec<u32> = Vec::new();
    let (mut x, mut y) = (0usize, 0usize);
    let mut i = 0usize;
    while i < data.len() {
        match data[i] {
            b'"' => {
                i += 1;
                // Pan;Pad;Ph;Pv
                let (_, ni) = number(data, i);
                i = ni;
                if data.get(i) == Some(&b';') {
                    i += 1;
                    let (_, ni) = number(data, i);
                    i = ni;
                }
                if data.get(i) == Some(&b';') {
                    i += 1;
                    let (ph, ni) = number(data, i);
                    i = ni;
                    if data.get(i) == Some(&b';') {
                        i += 1;
                        let (pv, ni) = number(data, i);
                        i = ni;
                        let _ = pv;
                    }
                    if ph > 0 {
                        let need_h = h.max(6);
                        grow(&mut buf, &mut w, &mut h, ph as usize, need_h, max_pixels)?;
                    }
                }
            }
            b'#' => {
                i += 1;
                let (pc, ni) = number(data, i);
                i = ni;
                if data.get(i) == Some(&b';') {
                    i += 1;
                    let (pu, ni) = number(data, i);
                    i = ni;
                    let mut v = [0u32; 3];
                    for slot in v.iter_mut() {
                        if data.get(i) == Some(&b';') {
                            i += 1;
                        }
                        let (n, ni) = number(data, i);
                        i = ni;
                        *slot = n;
                    }
                    if pc < 256 {
                        palette[pc as usize] = match pu {
                            1 => hls_to_rgb(v[0], v[1], v[2]),
                            _ => rgb(v[0], v[1], v[2]),
                        };
                    }
                }
                cur = pc.min(255) as u8;
            }
            b'!' => {
                i += 1;
                let (n, ni) = number(data, i);
                i = ni;
                if let Some(&c) = data.get(i) {
                    if (b'?'..=b'~').contains(&c) {
                        let bits = c - 0x3f;
                        for _ in 0..n.max(1) {
                            put(
                                &mut buf,
                                &mut w,
                                &mut h,
                                x,
                                y,
                                palette[cur as usize],
                                max_pixels,
                            )?;
                            for bit in 0..6 {
                                if bits & (1 << bit) != 0 {
                                    put(
                                        &mut buf,
                                        &mut w,
                                        &mut h,
                                        x,
                                        y + bit as usize,
                                        palette[cur as usize],
                                        max_pixels,
                                    )?;
                                }
                            }
                            x += 1;
                        }
                        i += 1;
                    }
                }
            }
            b'$' => {
                x = 0;
                i += 1;
            }
            b'-' => {
                x = 0;
                y += 6;
                i += 1;
            }
            c if (b'?'..=b'~').contains(&c) => {
                let bits = c - 0x3f;
                for bit in 0..6 {
                    if bits & (1 << bit) != 0 {
                        put(
                            &mut buf,
                            &mut w,
                            &mut h,
                            x,
                            y + bit as usize,
                            palette[cur as usize],
                            max_pixels,
                        )?;
                    }
                }
                x += 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    if w == 0 || h == 0 {
        return None;
    }
    // Sixel images are whole 6-pixel bands.
    if h % 6 != 0 {
        let need_h = h.div_ceil(6) * 6;
        let need_w = w;
        grow(&mut buf, &mut w, &mut h, need_w, need_h, max_pixels)?;
    }
    if w * h > max_pixels {
        return None;
    }
    let mut rgba = Vec::with_capacity(w * h * 4);
    for px in &buf {
        let (r, g, b, a) = unpack(*px);
        let a = if a == 0 && !transparent { 255 } else { a };
        rgba.extend_from_slice(&[r, g, b, a]);
    }
    buf.clear();
    Some(Image {
        width: w as u32,
        height: h as u32,
        rgba,
    })
}

fn put(
    buf: &mut Vec<u32>,
    w: &mut usize,
    h: &mut usize,
    x: usize,
    y: usize,
    color: u32,
    max_pixels: usize,
) -> Option<()> {
    grow(buf, w, h, x + 1, y + 1, max_pixels)?;
    buf[y * *w + x] = color;
    Some(())
}

fn grow(
    buf: &mut Vec<u32>,
    w: &mut usize,
    h: &mut usize,
    need_w: usize,
    need_h: usize,
    max_pixels: usize,
) -> Option<()> {
    if need_w <= *w && need_h <= *h {
        return Some(());
    }
    let nw = (*w).max(need_w);
    let nh = (*h).max(need_h);
    if nw * nh > max_pixels {
        return None;
    }
    let mut next = vec![0u32; nw * nh];
    for y in 0..*h {
        next[y * nw..y * nw + *w].copy_from_slice(&buf[y * *w..y * *w + *w]);
    }
    *buf = next;
    *w = nw;
    *h = nh;
    Some(())
}

fn number(s: &[u8], mut i: usize) -> (u32, usize) {
    let mut n = 0u32;
    while i < s.len() && s[i].is_ascii_digit() {
        n = n.saturating_mul(10).saturating_add((s[i] - b'0') as u32);
        i += 1;
    }
    (n, i)
}

fn rgb(r: u32, g: u32, b: u32) -> u32 {
    let f = |v: u32| ((v.min(100)) * 255 / 100) as u8;
    pack(f(r), f(g), f(b), 255)
}

/// HLS (0-360, 0-100, 0-100) → RGB.
fn hls_to_rgb(h: u32, l: u32, s: u32) -> u32 {
    let h = (h % 360) as f32 / 360.0;
    let l = (l.min(100) as f32) / 100.0;
    let s = (s.min(100) as f32) / 100.0;
    let (r, g, b) = hue_to_rgb(h, l, s);
    pack((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255)
}

fn hue_to_rgb(h: f32, l: f32, s: f32) -> (f32, f32, f32) {
    if s == 0.0 {
        return (l, l, l);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    (t(p, q, h + 1.0 / 3.0), t(p, q, h), t(p, q, h - 1.0 / 3.0))
}

fn t(p: f32, q: f32, mut t: f32) -> f32 {
    if t < 0.0 {
        t += 1.0;
    }
    if t > 1.0 {
        t -= 1.0;
    }
    if t < 1.0 / 6.0 {
        p + (q - p) * 6.0 * t
    } else if t < 1.0 / 2.0 {
        q
    } else if t < 2.0 / 3.0 {
        p + (q - p) * (2.0 / 3.0 - t) * 6.0
    } else {
        p
    }
}

fn pack(r: u8, g: u8, b: u8, a: u8) -> u32 {
    ((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | a as u32
}

fn unpack(px: u32) -> (u8, u8, u8, u8) {
    (
        (px >> 24) as u8,
        (px >> 16) as u8,
        (px >> 8) as u8,
        px as u8,
    )
}

fn default_palette() -> [u32; 256] {
    let mut p = [pack(0, 0, 0, 255); 256];
    const ANSI: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (205, 0, 0),
        (0, 205, 0),
        (205, 205, 0),
        (0, 0, 238),
        (205, 0, 205),
        (0, 205, 205),
        (229, 229, 229),
        (127, 127, 127),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (92, 92, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];
    for (i, &(r, g, b)) in ANSI.iter().enumerate() {
        p[i] = pack(r, g, b, 255);
    }
    // A grayscale ramp for the rest (mostly unused when a palette is sent).
    for (i, slot) in p.iter_mut().enumerate().skip(16) {
        let v = ((i - 16) * 255 / 239) as u8;
        *slot = pack(v, v, v, 255);
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_single_band() {
        // Define color 0 as pure red, select it, then set the top row of a band.
        let data = b"#0;2;100;0;0#0@";
        let img = decode(data, false, 1_000_000).unwrap();
        assert_eq!((img.width, img.height), (1, 6));
        assert_eq!(&img.rgba[..4], &[255, 0, 0, 255]); // top pixel set
        assert_eq!(&img.rgba[4..8], &[0, 0, 0, 255]); // next row black (opaque)
    }

    #[test]
    fn repeat_and_carriage_return() {
        let data = b"#0;2;0;100;0#0!2@$-";
        let img = decode(data, true, 1_000_000).unwrap();
        assert_eq!((img.width, img.height), (2, 6));
        // transparent background: an unset pixel stays alpha 0
        assert_eq!(img.rgba[3], 255, "first (set) pixel alpha");
        assert_eq!(img.rgba[8 + 3], 0, "row 1 pixel alpha stays transparent");
    }

    #[test]
    fn rejects_oversized() {
        assert!(decode(b"#0;2;0;0;0#0!10@", false, 4).is_none());
    }
}
