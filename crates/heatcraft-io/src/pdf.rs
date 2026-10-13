//! PDF floor plan import: the first page is rasterized to PNG with the
//! pure-Rust `hayro` renderer.
//!
//! Limitations (inherited from hayro): encrypted PDFs, blend modes,
//! knockout groups and a few rarely used features are not supported.
//! If a plan renders incorrectly, export it to PNG and import that instead.

use crate::error::IoError;
use crate::plan_image::MAX_SIDE_PX;
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{render, PixmapSettings, RenderCache, RenderSettings};

/// Longest side of the rasterized page, in pixels. High enough to keep
/// thin walls and labels legible when zooming in.
pub const TARGET_SIDE_PX: f32 = 4096.0;

pub struct RenderedPdf {
    pub png: Vec<u8>,
    pub page_count: usize,
}

/// Rasterizes the first page of a PDF to PNG bytes.
pub fn rasterize_first_page(bytes: Vec<u8>) -> Result<RenderedPdf, IoError> {
    let pdf = Pdf::new(bytes).map_err(|e| IoError::Format(format!("cannot read PDF: {e:?}")))?;
    let pages = pdf.pages();
    let page_count = pages.len();
    let page = pages
        .first()
        .ok_or_else(|| IoError::Format("the PDF has no pages".into()))?;
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

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A minimal one-page PDF (200 × 100 pt) with a black rectangle,
    /// built with a correct cross-reference table.
    pub(crate) fn sample_pdf() -> Vec<u8> {
        let content = "0 0 0 rg 20 20 100 50 re f";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Contents 4 0 R >>".to_owned(),
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
