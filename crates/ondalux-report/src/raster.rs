//! Minimal anti-aliased 2D drawing on an RGBA image: rectangles, thick
//! lines, circles and text (Ubuntu font shipped with egui).

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use image::{Rgba, RgbaImage};

pub type Color = [u8; 4];

pub const WHITE: Color = [255, 255, 255, 255];
pub const BLACK: Color = [20, 20, 25, 255];

pub fn font() -> FontRef<'static> {
    FontRef::try_from_slice(epaint_default_fonts::UBUNTU_LIGHT).expect("bundled font is valid")
}

pub struct Canvas {
    pub img: RgbaImage,
    font: FontRef<'static>,
}

impl Canvas {
    pub fn new(img: RgbaImage) -> Self {
        Self { img, font: font() }
    }

    pub fn width(&self) -> u32 {
        self.img.width()
    }

    pub fn height(&self) -> u32 {
        self.img.height()
    }

    /// Alpha-blends `c` (with extra `coverage` 0–1) over pixel (x, y).
    pub fn blend(&mut self, x: i32, y: i32, c: Color, coverage: f32) {
        if x < 0 || y < 0 || x >= self.img.width() as i32 || y >= self.img.height() as i32 {
            return;
        }
        let a = (c[3] as f32 / 255.0) * coverage.clamp(0.0, 1.0);
        if a <= 0.0 {
            return;
        }
        let p = self.img.get_pixel_mut(x as u32, y as u32);
        for (dst, src) in p.0.iter_mut().zip(c).take(3) {
            *dst = (*dst as f32 * (1.0 - a) + src as f32 * a).round() as u8;
        }
        p.0[3] = (p.0[3] as f32 + (255.0 - p.0[3] as f32) * a).round() as u8;
    }

    pub fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, c: Color) {
        let (x0, y0) = (x.floor() as i32, y.floor() as i32);
        let (x1, y1) = ((x + w).ceil() as i32, (y + h).ceil() as i32);
        for py in y0..y1 {
            for px in x0..x1 {
                // Partial coverage on the edges.
                let cx = ((px as f32 + 1.0).min(x + w) - (px as f32).max(x)).clamp(0.0, 1.0);
                let cy = ((py as f32 + 1.0).min(y + h) - (py as f32).max(y)).clamp(0.0, 1.0);
                self.blend(px, py, c, cx * cy);
            }
        }
    }

    pub fn stroke_rect(&mut self, x: f32, y: f32, w: f32, h: f32, width: f32, c: Color) {
        self.line(x, y, x + w, y, width, c);
        self.line(x + w, y, x + w, y + h, width, c);
        self.line(x + w, y + h, x, y + h, width, c);
        self.line(x, y + h, x, y, width, c);
    }

    /// Thick anti-aliased line segment with round caps.
    pub fn line(&mut self, ax: f32, ay: f32, bx: f32, by: f32, width: f32, c: Color) {
        let r = width / 2.0;
        let (x0, x1) = ((ax.min(bx) - r - 1.0) as i32, (ax.max(bx) + r + 1.0) as i32);
        let (y0, y1) = ((ay.min(by) - r - 1.0) as i32, (ay.max(by) + r + 1.0) as i32);
        let (dx, dy) = (bx - ax, by - ay);
        let len2 = dx * dx + dy * dy;
        for py in y0.max(0)..=y1.min(self.img.height() as i32 - 1) {
            for px in x0.max(0)..=x1.min(self.img.width() as i32 - 1) {
                let (qx, qy) = (px as f32 + 0.5, py as f32 + 0.5);
                let t = if len2 > 0.0 {
                    (((qx - ax) * dx + (qy - ay) * dy) / len2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let d = ((qx - ax - t * dx).powi(2) + (qy - ay - t * dy).powi(2)).sqrt();
                self.blend(px, py, c, r + 0.5 - d);
            }
        }
    }

    pub fn fill_circle(&mut self, cx: f32, cy: f32, radius: f32, c: Color) {
        for py in (cy - radius - 1.0) as i32..=(cy + radius + 1.0) as i32 {
            for px in (cx - radius - 1.0) as i32..=(cx + radius + 1.0) as i32 {
                let d = ((px as f32 + 0.5 - cx).powi(2) + (py as f32 + 0.5 - cy).powi(2)).sqrt();
                self.blend(px, py, c, radius + 0.5 - d);
            }
        }
    }

    pub fn stroke_circle(&mut self, cx: f32, cy: f32, radius: f32, width: f32, c: Color) {
        let h = width / 2.0;
        for py in (cy - radius - h - 1.0) as i32..=(cy + radius + h + 1.0) as i32 {
            for px in (cx - radius - h - 1.0) as i32..=(cx + radius + h + 1.0) as i32 {
                let d = ((px as f32 + 0.5 - cx).powi(2) + (py as f32 + 0.5 - cy).powi(2)).sqrt();
                self.blend(px, py, c, h + 0.5 - (d - radius).abs());
            }
        }
    }

    /// Width and height of `text` at `size` pixels.
    pub fn text_size(&self, text: &str, size: f32) -> (f32, f32) {
        let f = self.font.as_scaled(PxScale::from(size));
        let mut w = 0.0;
        let mut prev = None;
        for ch in text.chars() {
            let id = f.glyph_id(ch);
            if let Some(p) = prev {
                w += f.kern(p, id);
            }
            w += f.h_advance(id);
            prev = Some(id);
        }
        (w, f.ascent() - f.descent())
    }

    /// Draws `text` with its top-left corner at (x, y).
    pub fn text(&mut self, x: f32, y: f32, text: &str, size: f32, c: Color) {
        let scale = PxScale::from(size);
        let font = self.font.clone();
        let f = font.as_scaled(scale);
        let baseline = y + f.ascent();
        let mut pen = x;
        let mut prev = None;
        for ch in text.chars() {
            let id = f.glyph_id(ch);
            if let Some(p) = prev {
                pen += f.kern(p, id);
            }
            let glyph = id.with_scale_and_position(scale, ab_glyph::point(pen, baseline));
            pen += f.h_advance(id);
            prev = Some(id);
            if let Some(outline) = font.outline_glyph(glyph) {
                let b = outline.px_bounds();
                outline.draw(|gx, gy, cov| {
                    self.blend(
                        b.min.x as i32 + gx as i32,
                        b.min.y as i32 + gy as i32,
                        c,
                        cov,
                    );
                });
            }
        }
    }

    /// Text on a rounded-looking dark box, for labels over the map.
    pub fn label(&mut self, x: f32, y: f32, text: &str, size: f32) {
        let (w, h) = self.text_size(text, size);
        self.fill_rect(x - 3.0, y - 2.0, w + 6.0, h + 4.0, [0, 0, 0, 175]);
        self.text(x, y, text, size, WHITE);
    }
}

pub fn into_rgba(c: [u8; 3], alpha: u8) -> Rgba<u8> {
    Rgba([c[0], c[1], c[2], alpha])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank() -> Canvas {
        Canvas::new(RgbaImage::from_pixel(60, 40, Rgba(WHITE)))
    }

    #[test]
    fn shapes_and_text_change_pixels() {
        let mut c = blank();
        c.line(5.0, 5.0, 55.0, 5.0, 3.0, BLACK);
        assert!(c.img.get_pixel(30, 5).0[0] < 50);
        assert_eq!(c.img.get_pixel(30, 20).0, WHITE);
        c.fill_circle(30.0, 25.0, 5.0, [255, 0, 0, 255]);
        assert_eq!(c.img.get_pixel(30, 25).0, [255, 0, 0, 255]);
        let before = c.img.clone();
        c.text(2.0, 28.0, "Ap", 12.0, BLACK);
        assert_ne!(c.img, before);
        let (w, h) = c.text_size("Ap", 12.0);
        assert!(w > 8.0 && w < 30.0 && h > 8.0, "{w} {h}");
    }

    #[test]
    fn blending_respects_alpha() {
        let mut c = blank();
        c.fill_rect(0.0, 0.0, 10.0, 10.0, [0, 0, 0, 128]);
        let v = c.img.get_pixel(5, 5).0[0];
        assert!((120..=135).contains(&v), "{v}");
    }
}
