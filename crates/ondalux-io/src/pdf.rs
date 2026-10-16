//! PDF floor plan import: pages are rasterized to PNG with the pure-Rust
//! `hayro` renderer (one floor per page for multi-page plans).
//!
//! Limitations (inherited from hayro): encrypted PDFs, blend modes,
//! knockout groups and a few rarely used features are not supported.
//! If a plan renders incorrectly, export it to PNG and import that instead.

use crate::error::IoError;
use crate::plan_image::MAX_SIDE_PX;
use hayro::hayro_interpret::font::GlyphRun;
use hayro::hayro_interpret::util::TransformExt;
use hayro::hayro_interpret::{
    interpret_page, BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, FillRule, Image,
    ImageDrawProps, InterpreterCache, InterpreterSettings, Paint, SoftMask,
};
use hayro::hayro_syntax::Pdf;
use hayro::kurbo::{self, Affine, BezPath, PathEl, Point, Rect};
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{render, PixmapSettings, RenderCache, RenderSettings};
use ondalux_core::walldetect::Mask;

/// Longest side of the rasterized page, in pixels. High enough to keep
/// thin walls and labels legible when zooming in.
pub const TARGET_SIDE_PX: f32 = 4096.0;

pub struct RenderedPdf {
    pub png: Vec<u8>,
    pub page_count: usize,
}

/// Rasterizes the first page of a PDF to PNG bytes.
pub fn rasterize_first_page(bytes: Vec<u8>) -> Result<RenderedPdf, IoError> {
    rasterize_page(bytes, 0)
}

/// Number of pages of a PDF.
pub fn page_count(bytes: Vec<u8>) -> Result<usize, IoError> {
    let pdf = Pdf::new(bytes).map_err(|e| IoError::Format(format!("cannot read PDF: {e:?}")))?;
    Ok(pdf.pages().len())
}

/// Rasterizes page `index` (0-based) of a PDF to PNG bytes.
pub fn rasterize_page(bytes: Vec<u8>, index: usize) -> Result<RenderedPdf, IoError> {
    let pdf = Pdf::new(bytes).map_err(|e| IoError::Format(format!("cannot read PDF: {e:?}")))?;
    let pages = pdf.pages();
    let page_count = pages.len();
    let page = pages
        .get(index)
        .ok_or_else(|| IoError::Format(format!("the PDF has no page {}", index + 1)))?;
    let (w, h) = page.render_dimensions();
    if w <= 0.0 || h <= 0.0 {
        return Err(IoError::Format("the PDF page has an empty size".into()));
    }
    let scale = (TARGET_SIDE_PX / w.max(h)).min(MAX_SIDE_PX as f32 / w.max(h));
    let pixmap = render(
        page,
        &RenderCache::new(),
        &InterpreterSettings::default(),
        &RenderSettings::default(),
        &PixmapSettings {
            x_scale: scale,
            y_scale: scale,
            bg_color: WHITE,
        },
    );
    let png = pixmap
        .into_png()
        .map_err(|e| IoError::Format(format!("cannot encode rendered PDF page: {e}")))?;
    Ok(RenderedPdf { png, page_count })
}

/// Line-art mask of page `index` for wall detection: the page is drawn
/// again with only its dark strokes and fills, leaving out text, images
/// and dashed lines (hidden or overhead elements), at `width × height`
/// (the plan image size times the detection scale).
pub fn vector_mask(
    bytes: Vec<u8>,
    index: usize,
    width: usize,
    height: usize,
) -> Result<Mask, IoError> {
    let pdf = Pdf::new(bytes).map_err(|e| IoError::Format(format!("cannot read PDF: {e:?}")))?;
    let pages = pdf.pages();
    let page = pages
        .get(index)
        .ok_or_else(|| IoError::Format(format!("the PDF has no page {}", index + 1)))?;
    let (w, h) = page.render_dimensions();
    if w <= 0.0 || h <= 0.0 || width == 0 || height == 0 {
        return Err(IoError::Format("the PDF page has an empty size".into()));
    }
    let transform = Affine::scale_non_uniform(width as f64 / w as f64, height as f64 / h as f64)
        * page.initial_transform(true).to_kurbo();
    let cache = InterpreterCache::new();
    let mut context = Context::new(
        transform,
        Rect::new(0.0, 0.0, width as f64, height as f64),
        &cache,
        page.xref(),
        InterpreterSettings::default(),
    );
    let mut device = MaskDevice {
        mask: Mask::new(width, height),
    };
    interpret_page(page, &mut context, &mut device);
    Ok(device.mask)
}

/// Collects dark line art into a mask.
struct MaskDevice {
    mask: Mask,
}

/// Luminance of a paint, `None` for patterns and shadings.
fn paint_luma(paint: &Paint<'_>) -> Option<f32> {
    match paint {
        Paint::Color(c) => {
            let [r, g, b, a] = c.to_rgba().to_rgba8();
            if a < 40 {
                return None;
            }
            Some(0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32)
        }
        Paint::Pattern(_) => None,
    }
}

impl MaskDevice {
    /// Straight line segments of `path` after `transform`.
    fn polylines(path: &BezPath, transform: Affine) -> Vec<Vec<Point>> {
        let mut out: Vec<Vec<Point>> = Vec::new();
        kurbo::flatten(transform * path.clone(), 0.25, |el| match el {
            PathEl::MoveTo(p) => out.push(vec![p]),
            PathEl::LineTo(p) => {
                if let Some(last) = out.last_mut() {
                    last.push(p);
                }
            }
            PathEl::ClosePath => {
                if let Some(last) = out.last_mut() {
                    if let Some(&first) = last.first() {
                        last.push(first);
                    }
                }
            }
            _ => {}
        });
        out
    }

    fn stamp(&mut self, x: f64, y: f64, r: f64, ink: bool) {
        if r < 0.75 {
            // Hairline: exactly one pixel.
            let (xi, yi) = (x.floor(), y.floor());
            if xi >= 0.0
                && yi >= 0.0
                && (xi as usize) < self.mask.width
                && (yi as usize) < self.mask.height
            {
                self.mask.set(xi as usize, yi as usize, ink);
            }
            return;
        }
        let (w, h) = (self.mask.width as f64, self.mask.height as f64);
        let (x0, x1) = ((x - r).floor().max(0.0), (x + r).ceil().min(w - 1.0));
        let (y0, y1) = ((y - r).floor().max(0.0), (y + r).ceil().min(h - 1.0));
        if x0 > x1 || y0 > y1 {
            return;
        }
        for yy in y0 as usize..=y1 as usize {
            for xx in x0 as usize..=x1 as usize {
                self.mask.set(xx, yy, ink);
            }
        }
    }

    fn stroke(&mut self, lines: &[Vec<Point>], width: f64, ink: bool) {
        let r = width / 2.0;
        for line in lines {
            for pair in line.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                let len = a.distance(b);
                let steps = (len / 0.7).ceil().max(1.0) as usize;
                for i in 0..=steps {
                    let t = i as f64 / steps as f64;
                    self.stamp(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t, r, ink);
                }
            }
        }
    }

    /// Scanline fill with the non-zero or even-odd rule.
    fn fill(&mut self, lines: &[Vec<Point>], even_odd: bool, ink: bool) {
        let mut edges: Vec<(Point, Point)> = Vec::new();
        for line in lines {
            for pair in line.windows(2) {
                edges.push((pair[0], pair[1]));
            }
            if let (Some(&f), Some(&l)) = (line.first(), line.last()) {
                if f != l {
                    edges.push((l, f));
                }
            }
        }
        if edges.is_empty() {
            return;
        }
        let (mut ymin, mut ymax) = (f64::MAX, f64::MIN);
        for (a, b) in &edges {
            ymin = ymin.min(a.y.min(b.y));
            ymax = ymax.max(a.y.max(b.y));
        }
        let h = self.mask.height as f64;
        let w = self.mask.width as f64;
        let (y0, y1) = (ymin.floor().max(0.0) as usize, ymax.ceil().min(h) as usize);
        let mut xs: Vec<(f64, i32)> = Vec::new();
        for y in y0..y1 {
            let sy = y as f64 + 0.5;
            xs.clear();
            for (a, b) in &edges {
                if (a.y <= sy && b.y > sy) || (b.y <= sy && a.y > sy) {
                    let t = (sy - a.y) / (b.y - a.y);
                    xs.push((a.x + (b.x - a.x) * t, if b.y > a.y { 1 } else { -1 }));
                }
            }
            xs.sort_by(|p, q| p.0.total_cmp(&q.0));
            let mut winding = 0;
            for k in 0..xs.len() {
                winding += if even_odd { 1 } else { xs[k].1 };
                let inside = if even_odd {
                    winding % 2 != 0
                } else {
                    winding != 0
                };
                if inside && k + 1 < xs.len() {
                    let (x0, x1) = (xs[k].0.max(0.0), xs[k + 1].0.min(w));
                    if x1 > x0 {
                        for x in x0.round() as usize..(x1.round() as usize).min(self.mask.width) {
                            self.mask.set(x, y, ink);
                        }
                    }
                }
            }
        }
    }
}

impl<'a> Device<'a> for MaskDevice {
    fn draw_path(&mut self, path: &BezPath, props: DrawProps<'a>, draw_mode: &DrawMode) {
        let Some(luma) = paint_luma(&props.paint) else {
            return;
        };
        // Dark paint draws ink; white paint erases (masks used in plans).
        let ink = if luma < 160.0 {
            true
        } else if luma > 235.0 {
            false
        } else {
            return;
        };
        let lines = Self::polylines(path, props.transform);
        let scale = props.transform.determinant().abs().sqrt();
        let stroke = |dev: &mut Self, s: &hayro::hayro_interpret::StrokeProps| {
            if s.dash_array.iter().any(|d| *d > 0.0) {
                return;
            }
            dev.stroke(&lines, s.line_width as f64 * scale, ink);
        };
        match draw_mode {
            DrawMode::Fill(rule) => self.fill(&lines, matches!(rule, FillRule::EvenOdd), ink),
            DrawMode::Stroke(s) => stroke(self, s),
            DrawMode::FillAndStroke(rule, s) => {
                self.fill(&lines, matches!(rule, FillRule::EvenOdd), ink);
                stroke(self, s);
            }
            DrawMode::Invisible => {}
        }
    }
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_glyph_run(&mut self, _: &GlyphRun<'_, 'a>, _: DrawProps<'a>, _: &DrawMode) {}
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A minimal one-page PDF (200 × 100 pt) with a black rectangle,
    /// built with a correct cross-reference table.
    pub(crate) fn sample_pdf() -> Vec<u8> {
        pdf_with_content("0 0 0 rg 20 20 100 50 re f", 200, 100)
    }

    /// One-page PDF of `w × h` pt with the given content stream.
    pub(crate) fn pdf_with_content(content: &str, w: u32, h: u32) -> Vec<u8> {
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Contents 4 0 R >>"),
            format!(
                "<< /Length {} >>\nstream\n{content}\nendstream",
                content.len()
            ),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, obj) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{obj}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for off in offsets {
            out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }

    /// Two pages: 200 × 100 pt with a rectangle on the left, then 300 ×
    /// 100 pt with one on the right.
    pub(crate) fn two_page_pdf() -> Vec<u8> {
        let c1 = "0 0 0 rg 10 10 40 80 re f";
        let c2 = "0 0 0 rg 250 10 40 80 re f";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            "<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>".to_owned(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Contents 4 0 R >>".to_owned(),
            format!(
                "<< /Length {} >>
stream
{c1}
endstream",
                c1.len()
            ),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 100] /Contents 6 0 R >>".to_owned(),
            format!(
                "<< /Length {} >>
stream
{c2}
endstream",
                c2.len()
            ),
        ];
        let mut out = b"%PDF-1.4
"
        .to_vec();
        let mut offsets = Vec::new();
        for (i, obj) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(
                format!(
                    "{} 0 obj
{obj}
endobj
",
                    i + 1
                )
                .as_bytes(),
            );
        }
        let xref = out.len();
        out.extend_from_slice(
            format!(
                "xref
0 {}
0000000000 65535 f 
",
                objects.len() + 1
            )
            .as_bytes(),
        );
        for off in offsets {
            out.extend_from_slice(
                format!(
                    "{off:010} 00000 n 
"
                )
                .as_bytes(),
            );
        }
        out.extend_from_slice(
            format!(
                "trailer
<< /Size {} /Root 1 0 R >>
startxref
{xref}
%%EOF
",
                objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }

    #[test]
    fn renders_every_page() {
        let pdf = two_page_pdf();
        assert_eq!(page_count(pdf.clone()).unwrap(), 2);
        let png = rasterize_page(pdf.clone(), 1).unwrap().png;
        let img = image::load_from_memory(&png).unwrap().to_rgba8();
        // The second page is wider (3:1) and its ink is on the right.
        assert!((img.width() as f32 / img.height() as f32 - 3.0).abs() < 0.05);
        let y = img.height() / 2;
        assert!(img.get_pixel(img.width() * 9 / 10, y)[0] < 50);
        assert!(img.get_pixel(img.width() / 10, y)[0] > 200);
        // The vector mask follows the page too.
        let m = vector_mask(pdf.clone(), 1, 300, 100).unwrap();
        assert!(m.get(270, 50) && !m.get(30, 50));
        assert!(rasterize_page(pdf, 2).is_err());
    }

    #[test]
    fn rasterizes_first_page() {
        let r = rasterize_first_page(sample_pdf()).unwrap();
        assert_eq!(r.page_count, 1);
        let img = image::load_from_memory_with_format(&r.png, image::ImageFormat::Png)
            .unwrap()
            .into_rgba8();
        // 200 × 100 pt scaled so the long side is TARGET_SIDE_PX.
        assert_eq!(img.width(), TARGET_SIDE_PX as u32);
        assert_eq!(img.height(), TARGET_SIDE_PX as u32 / 2);
        // PDF y grows upwards: the rectangle (y 20–70 pt) is in the lower part.
        let s = TARGET_SIDE_PX / 200.0;
        let inside = img.get_pixel((60.0 * s) as u32, ((100.0 - 45.0) * s) as u32);
        let outside = img.get_pixel((180.0 * s) as u32, (10.0 * s) as u32);
        assert!(inside[0] < 30, "{inside:?}");
        assert!(outside[0] > 225, "{outside:?}");
    }

    #[test]
    fn garbage_is_rejected() {
        assert!(rasterize_first_page(b"not a pdf".to_vec()).is_err());
    }
}
