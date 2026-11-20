//! Generated status image served before the first good frame (e.g. "cannot reach Home Assistant").
use crate::image::{encode_1bit, to_bits, Dither, Params};
use font8x8::{UnicodeFonts, BASIC_FONTS};

/// Draw `lines` (first line large) as black text on white, then apply the normal 1-bit pipeline so the
/// placeholder has the same polarity as real frames.
pub fn render(lines: &[String], width: u32, height: u32, invert: bool) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let mut g = vec![255u8; w * h];
    let mut y = 24usize;
    for (i, line) in lines.iter().enumerate() {
        let scale = if i == 0 { 3 } else { 2 };
        let max_chars = (w.saturating_sub(48)) / (8 * scale);
        for chunk in wrap(line, max_chars.max(1)) {
            if y + 8 * scale > h {
                break;
            }
            draw_text(&mut g, w, 24, y, &chunk, scale);
            y += 8 * scale + 6;
        }
        y += if i == 0 { 14 } else { 4 };
    }
    let p = Params {
        width,
        height,
        top: 0,
        threshold: 128,
        invert,
        dither: Dither::None,
    };
    encode_1bit(&to_bits(&g, &p), width, height)
}

fn wrap(s: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for word in s.split_whitespace() {
        let mut word = word.to_string();
        while word.chars().count() > max {
            let head: String = word.chars().take(max).collect();
            word = word.chars().skip(max).collect();
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            out.push(head);
        }
        if !cur.is_empty() && cur.chars().count() + 1 + word.chars().count() > max {
            out.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(&word);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn draw_text(g: &mut [u8], w: usize, x0: usize, y0: usize, text: &str, scale: usize) {
    for (ci, ch) in text.chars().enumerate() {
        let glyph = BASIC_FONTS.get(ch).or_else(|| BASIC_FONTS.get('?')).unwrap_or([0; 8]);
        for (row, bits) in glyph.iter().enumerate() {
            for col in 0..8 {
                if bits & (1 << col) == 0 {
                    continue;
                }
                for dy in 0..scale {
                    for dx in 0..scale {
                        let x = x0 + (ci * 8 + col) * scale + dx;
                        let y = y0 + row * scale + dy;
                        if x < w && y * w + x < g.len() {
                            g[y * w + x] = 0;
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_is_a_valid_1bit_png_with_text() {
        let png = render(
            &["Cannot reach Home Assistant".into(), "detail".into()],
            800,
            480,
            false,
        );
        let mut r = png::Decoder::new(png.as_slice()).read_info().unwrap();
        let info = r.info().clone();
        assert_eq!((info.width, info.height), (800, 480));
        assert_eq!(info.bit_depth, png::BitDepth::One);
        let mut buf = vec![0; r.output_buffer_size()];
        r.next_frame(&mut buf).unwrap();
        // not inverted: background white (bit 1), some black text pixels (bit 0)
        let zeros: u32 = buf.iter().map(|b| b.count_zeros()).sum();
        assert!(zeros > 500 && zeros < 800 * 480 / 4, "text pixels: {zeros}");
    }

    #[test]
    fn wrap_breaks_long_lines() {
        assert_eq!(wrap("aaa bbb ccc", 7), vec!["aaa bbb", "ccc"]);
        assert_eq!(wrap("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
    }
}
