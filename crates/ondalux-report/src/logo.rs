//! The Ondalux logo, drawn from code so one definition gives the window
//! icon, the welcome screen image and the Windows `.ico` (see `build.rs`,
//! which includes this file, so it must only use `std`).
//!
//! Design: a rounded blue tile with an access point (white dot) sending
//! three waves colored like the heat map scale: green near the AP, then
//! yellow, then orange where the signal gets weaker.

/// Colors in linear-ish sRGB floats (0..1).
type Rgb = [f32; 3];

const TILE_TOP: Rgb = rgb(0x25, 0x63, 0xEB);
const TILE_BOTTOM: Rgb = rgb(0x0B, 0x1E, 0x5B);
const WAVES: [Rgb; 3] = [
    rgb(0x22, 0xC5, 0x5E),
    rgb(0xFA, 0xCC, 0x15),
    rgb(0xF9, 0x73, 0x16),
];
const WHITE: Rgb = [1.0, 1.0, 1.0];

/// Wave center, radii and stroke width, in tile units (0..1).
const CENTER: (f32, f32) = (0.5, 0.68);
const RADII: [f32; 3] = [0.17, 0.30, 0.43];
const WAVE_WIDTH: f32 = 0.08;
/// Half opening angle of the waves, in degrees from "up".
const WAVE_HALF_ANGLE: f32 = 50.0;
const DOT_RADIUS: f32 = 0.075;

const fn rgb(r: u8, g: u8, b: u8) -> Rgb {
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0]
}

/// Signed distance to a rounded square centered in the tile.
fn tile_sdf(x: f32, y: f32) -> f32 {
    let half = 0.47;
    let radius = 0.2;
    let qx = (x - 0.5).abs() - (half - radius);
    let qy = (y - 0.5).abs() - (half - radius);
    let outside = qx.max(0.0).hypot(qy.max(0.0));
    outside + qx.max(qy).min(0.0) - radius
}

/// Signed distance to an arc of radius `r` opening upwards, round caps.
fn arc_sdf(x: f32, y: f32, r: f32) -> f32 {
    let (px, py) = (x - CENTER.0, y - CENTER.1);
    let angle = px.atan2(-py).to_degrees();
    if angle.abs() <= WAVE_HALF_ANGLE {
        (px.hypot(py) - r).abs() - WAVE_WIDTH / 2.0
    } else {
        let a = WAVE_HALF_ANGLE.to_radians().copysign(angle);
        let (ex, ey) = (r * a.sin(), -r * a.cos());
        (px - ex).hypot(py - ey) - WAVE_WIDTH / 2.0
    }
}

/// Coverage of a shape from its signed distance, antialiased over 1 px.
fn coverage(d: f32, size: f32) -> f32 {
    (0.5 - d * size).clamp(0.0, 1.0)
}

/// Premultiplied "over" compositing.
fn over(dst: &mut [f32; 4], color: Rgb, alpha: f32) {
    for i in 0..3 {
        dst[i] = color[i] * alpha + dst[i] * (1.0 - alpha);
    }
    dst[3] = alpha + dst[3] * (1.0 - alpha);
}

/// The logo as straight (not premultiplied) RGBA, `size × size`, row-major.
pub fn rgba(size: u32) -> Vec<u8> {
    let s = size as f32;
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for row in 0..size {
        for col in 0..size {
            let (x, y) = ((col as f32 + 0.5) / s, (row as f32 + 0.5) / s);
            let mut px = [0.0f32; 4];
            let tile = coverage(tile_sdf(x, y), s);
            if tile > 0.0 {
                let t = ((x + y) / 2.0).clamp(0.0, 1.0);
                let bg: Rgb =
                    std::array::from_fn(|i| TILE_TOP[i] + (TILE_BOTTOM[i] - TILE_TOP[i]) * t);
                over(&mut px, bg, tile);
                // Soft light around the AP.
                let d = (x - CENTER.0).hypot(y - CENTER.1);
                over(&mut px, WHITE, 0.28 * (-(d / 0.13).powi(2)).exp() * tile);
                for (r, color) in RADII.iter().zip(WAVES) {
                    over(&mut px, color, coverage(arc_sdf(x, y, *r), s) * tile);
                }
                over(&mut px, WHITE, coverage(d - DOT_RADIUS, s) * tile);
            }
            let a = px[3];
            for c in &px[..3] {
                let v = if a > 0.0 { c / a } else { 0.0 };
                out.push((v.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
            out.push((a * 255.0).round() as u8);
        }
    }
    out
}

/// Windows icon with the usual sizes, as 32-bit BMP entries (used by
/// `build.rs`).
#[cfg_attr(not(test), allow(dead_code))]
pub fn ico() -> Vec<u8> {
    const SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];
    let images: Vec<Vec<u8>> = SIZES.iter().map(|&s| bmp_entry(s)).collect();
    let mut out = Vec::new();
    // ICONDIR
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(SIZES.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * SIZES.len() as u32;
    for (s, img) in SIZES.iter().zip(&images) {
        let dim = if *s >= 256 { 0 } else { *s as u8 };
        out.extend_from_slice(&[dim, dim, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&(img.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for img in images {
        out.extend_from_slice(&img);
    }
    out
}

/// One icon image: BITMAPINFOHEADER, bottom-up BGRA pixels, empty AND mask.
#[cfg_attr(not(test), allow(dead_code))]
fn bmp_entry(size: u32) -> Vec<u8> {
    let pixels = rgba(size);
    let mask_row = size.div_ceil(32) * 4;
    let mut out = Vec::new();
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(size as i32).to_le_bytes());
    out.extend_from_slice(&(2 * size as i32).to_le_bytes()); // XOR + AND
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&(size * size * 4 + mask_row * size).to_le_bytes());
    out.extend_from_slice(&[0u8; 16]); // resolution and palette fields
    for row in (0..size).rev() {
        for col in 0..size {
            let i = ((row * size + col) * 4) as usize;
            out.extend_from_slice(&[pixels[i + 2], pixels[i + 1], pixels[i], pixels[i + 3]]);
        }
    }
    out.extend(std::iter::repeat_n(0u8, (mask_row * size) as usize));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corners_are_transparent_and_center_is_opaque() {
        let s = 64;
        let px = rgba(s);
        assert_eq!(px.len(), (s * s * 4) as usize);
        assert_eq!(px[3], 0, "top-left corner");
        let mid = ((s / 2 * s + s / 2) * 4) as usize;
        assert_eq!(px[mid + 3], 255);
    }

    /// Writes `logo.png` (512 px) into `$ONDALUX_LOGO_DIR`, for the docs:
    /// `ONDALUX_LOGO_DIR=docs cargo test -p ondalux-report export_logo -- --ignored`
    #[test]
    #[ignore]
    fn export_logo_png() {
        let dir = std::env::var("ONDALUX_LOGO_DIR").expect("set ONDALUX_LOGO_DIR");
        let s = 512;
        let img = image::RgbaImage::from_raw(s, s, rgba(s)).unwrap();
        img.save(std::path::Path::new(&dir).join("logo.png"))
            .unwrap();
    }

    #[test]
    fn ico_directory_is_consistent() {
        let ico = ico();
        assert_eq!(u16::from_le_bytes([ico[2], ico[3]]), 1);
        let n = u16::from_le_bytes([ico[4], ico[5]]) as usize;
        let last = 6 + 16 * (n - 1);
        let len = u32::from_le_bytes(ico[last + 8..last + 12].try_into().unwrap());
        let off = u32::from_le_bytes(ico[last + 12..last + 16].try_into().unwrap());
        assert_eq!((off + len) as usize, ico.len());
    }
}
