//! Frame -> 1-bit PNG pipeline: libvips/sharp-style B_W greyscale, `>= threshold` is white, optional invert.
use std::sync::OnceLock;

pub struct Params {
    pub width: u32,
    pub height: u32,
    pub top: u32,
    pub threshold: u8,
    pub invert: bool,
    pub dither: Dither,
}
#[derive(Clone, Copy, PartialEq)]
pub enum Dither {
    None,
    Atkinson,
    FloydSteinberg,
}
impl Dither {
    pub fn parse(s: &str) -> Dither {
        match s {
            "atkinson" => Dither::Atkinson,
            "floyd-steinberg" => Dither::FloydSteinberg,
            _ => Dither::None,
        }
    }
}

/// sRGB 8-bit -> linear (0..1)
fn lin_lut() -> &'static [f32; 256] {
    static L: OnceLock<[f32; 256]> = OnceLock::new();
    L.get_or_init(|| {
        let mut t = [0f32; 256];
        for (i, v) in t.iter_mut().enumerate() {
            let c = i as f32 / 255.0;
            *v = if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            };
        }
        t
    })
}
fn enc(y: f32) -> u8 {
    let c = if y <= 0.0031308 {
        12.92 * y
    } else {
        1.055 * y.powf(1.0 / 2.4) - 0.055
    };
    (c * 255.0).round().clamp(0.0, 255.0) as u8
}

/// Decoded frame -> greyscale of the crop window (row-major width*height bytes).
pub fn decode_grey(png_bytes: &[u8], p: &Params) -> Result<Vec<u8>, String> {
    let mut dec = png::Decoder::new(png_bytes);
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = dec.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let (fw, fh) = (info.width, info.height);
    let ch = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Grayscale => 1,
        _ => return Err("unexpected color type".into()),
    };
    if fw < p.width || fh < p.top + p.height {
        return Err(format!(
            "frame {fw}x{fh} smaller than crop {}x{}+{}",
            p.width, p.height, p.top
        ));
    }
    let lut = lin_lut();
    let mut g = vec![0u8; (p.width * p.height) as usize];
    for y in 0..p.height {
        let row = &buf[((y + p.top) * fw) as usize * ch..];
        for x in 0..p.width as usize {
            let px = &row[x * ch..];
            g[y as usize * p.width as usize + x] = if ch >= 3 {
                enc(0.2126 * lut[px[0] as usize] + 0.7152 * lut[px[1] as usize] + 0.0722 * lut[px[2] as usize])
            } else {
                px[0]
            };
        }
    }
    Ok(g)
}

/// Greyscale -> packed 1-bit rows (1 = white), applying threshold or error diffusion, then invert.
pub fn to_bits(grey: &[u8], p: &Params) -> Vec<u8> {
    let (w, h) = (p.width as usize, p.height as usize);
    let white: Vec<bool> = match p.dither {
        Dither::None => grey.iter().map(|&v| v >= p.threshold).collect(),
        d => {
            let mut e: Vec<f32> = grey.iter().map(|&v| v as f32).collect();
            let mut out = vec![false; w * h];
            for y in 0..h {
                for x in 0..w {
                    let i = y * w + x;
                    let old = e[i];
                    let wht = old >= 128.0; // error diffusion uses the mid-point; `threshold` applies to dither=none
                    let new = if wht { 255.0 } else { 0.0 };
                    out[i] = wht;
                    let err = old - new;
                    let mut add = |dx: isize, dy: usize, f: f32| {
                        let nx = x as isize + dx;
                        let ny = y + dy;
                        if nx >= 0 && (nx as usize) < w && ny < h {
                            e[ny * w + nx as usize] += err * f;
                        }
                    };
                    if d == Dither::Atkinson {
                        let f = 1.0 / 8.0;
                        add(1, 0, f);
                        add(2, 0, f);
                        add(-1, 1, f);
                        add(0, 1, f);
                        add(1, 1, f);
                        add(0, 2, f);
                    } else {
                        add(1, 0, 7.0 / 16.0);
                        add(-1, 1, 3.0 / 16.0);
                        add(0, 1, 5.0 / 16.0);
                        add(1, 1, 1.0 / 16.0);
                    }
                }
            }
            out
        }
    };
    let stride = w.div_ceil(8);
    let mut bits = vec![0u8; stride * h];
    for y in 0..h {
        for x in 0..w {
            if white[y * w + x] != p.invert {
                bits[y * stride + x / 8] |= 0x80 >> (x % 8);
            }
        }
    }
    bits
}

/// Packed bits -> PNG: colour type 3 (palette [black, white]), bit depth 1, non-interlaced (what sharp emits for 2-colour PNGs).
pub fn encode_1bit(bits: &[u8], w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 * 1024);
    {
        let mut e = png::Encoder::new(&mut out, w, h);
        e.set_color(png::ColorType::Indexed);
        e.set_depth(png::BitDepth::One);
        e.set_palette(vec![0, 0, 0, 255, 255, 255]);
        e.set_compression(png::Compression::Best);
        e.set_filter(png::FilterType::NoFilter);
        let mut wr = e.write_header().expect("png header");
        wr.write_image_data(bits).expect("png data");
    }
    out
}

pub fn encode_grey(grey: &[u8], w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, w, h);
        e.set_color(png::ColorType::Grayscale);
        e.set_depth(png::BitDepth::Eight);
        let mut wr = e.write_header().expect("png header");
        wr.write_image_data(grey).expect("png data");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(invert: bool, dither: Dither) -> Params {
        Params {
            width: 16,
            height: 2,
            top: 0,
            threshold: 220,
            invert,
            dither,
        }
    }

    fn rgb_png(w: u32, h: u32, px: &[[u8; 3]]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut e = png::Encoder::new(&mut out, w, h);
            e.set_color(png::ColorType::Rgb);
            e.set_depth(png::BitDepth::Eight);
            let mut wr = e.write_header().unwrap();
            wr.write_image_data(&px.iter().flatten().copied().collect::<Vec<u8>>())
                .unwrap();
        }
        out
    }

    #[test]
    fn threshold_is_inclusive_and_packs_msb_first() {
        let mut grey = vec![0u8; 32];
        grey[0] = 220; // exactly the threshold -> white
        grey[1] = 219; // just below -> black
        grey[9] = 255;
        let bits = to_bits(&grey, &params(false, Dither::None));
        assert_eq!(bits.len(), 4); // 2 bytes per 16-px row
        assert_eq!(bits[0], 0b1000_0000);
        assert_eq!(bits[1], 0b0100_0000);
        assert_eq!(&bits[2..], &[0, 0]);
    }

    #[test]
    fn invert_flips_every_pixel() {
        let grey: Vec<u8> = (0..32).map(|i| if i % 3 == 0 { 255 } else { 0 }).collect();
        let a = to_bits(&grey, &params(false, Dither::None));
        let b = to_bits(&grey, &params(true, Dither::None));
        for (x, y) in a.iter().zip(&b) {
            assert_eq!(*x, !*y);
        }
    }

    #[test]
    fn luminance_matches_libvips_bw() {
        // Linear-light Rec.709 luminance re-encoded to sRGB (what sharp/libvips B_W gives).
        let png = rgb_png(4, 1, &[[255, 255, 255], [0, 0, 0], [255, 0, 0], [255, 214, 0]]);
        let p = Params {
            width: 4,
            height: 1,
            top: 0,
            threshold: 220,
            invert: false,
            dither: Dither::None,
        };
        let g = decode_grey(&png, &p).unwrap();
        assert_eq!(g[0], 255);
        assert_eq!(g[1], 0);
        assert_eq!(g[2], 127); // pure red: Y=0.2126 -> sRGB 127
        assert!(g[3] > 210 && g[3] < 225, "yellow icon grey = {}", g[3]);
    }

    #[test]
    fn crop_skips_header_rows() {
        let mut px = vec![[0u8; 3]; 2 * 3];
        px[4] = [255, 255, 255]; // row 2, col 0
        let png = rgb_png(2, 3, &px);
        let p = Params {
            width: 2,
            height: 1,
            top: 2,
            threshold: 220,
            invert: false,
            dither: Dither::None,
        };
        assert_eq!(decode_grey(&png, &p).unwrap(), vec![255, 0]);
        let too_small = Params {
            width: 2,
            height: 2,
            top: 2,
            threshold: 220,
            invert: false,
            dither: Dither::None,
        };
        assert!(decode_grey(&png, &too_small).is_err());
    }

    #[test]
    fn atkinson_mid_grey_is_roughly_half_white() {
        let p = Params {
            width: 64,
            height: 64,
            top: 0,
            threshold: 220,
            invert: false,
            dither: Dither::Atkinson,
        };
        let bits = to_bits(&vec![128u8; 64 * 64], &p);
        let ones: u32 = bits.iter().map(|b| b.count_ones()).sum();
        assert!((1500..2600).contains(&ones), "white pixels {ones}");
    }

    #[test]
    fn encoded_png_matches_the_device_contract() {
        let p = Params {
            width: 800,
            height: 480,
            top: 0,
            threshold: 220,
            invert: true,
            dither: Dither::None,
        };
        let png = encode_1bit(&to_bits(&vec![255u8; 800 * 480], &p), 800, 480);
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        // IHDR: width, height, bit depth 1, colour type 3 (palette), interlace 0
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 800);
        assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), 480);
        assert_eq!((png[24], png[25], png[28]), (1, 3, 0));
        assert_eq!(&png[37..41], b"PLTE");
        assert_eq!(&png[41..47], &[0, 0, 0, 255, 255, 255]);
        // all white + invert -> every pixel black
        let mut r = png::Decoder::new(png.as_slice()).read_info().unwrap();
        let mut buf = vec![0; r.output_buffer_size()];
        r.next_frame(&mut buf).unwrap();
        assert!(buf.iter().all(|&b| b == 0));
    }
}
