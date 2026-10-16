//! Floor plan import (PNG, JPEG, first page of a PDF).

use crate::error::IoError;
use crate::project_file::PlanImage;
use std::path::Path;

/// Largest accepted side, in pixels. 8192 is the smallest maximum texture
/// size guaranteed by the GPU backends the UI runs on.
pub const MAX_SIDE_PX: u32 = 8192;

/// Decoded RGBA8 pixels, ready for display.
#[derive(Debug, Clone)]
pub struct DecodedPlan {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// A floor plan read from disk.
#[derive(Debug)]
pub struct ImportedPlan {
    pub image: PlanImage,
    pub decoded: DecodedPlan,
    /// The original file when it was a PDF (its line art is used to
    /// detect walls).
    pub source_pdf: Option<Vec<u8>>,
    /// Page of `source_pdf` this plan comes from (0-based).
    pub source_page: usize,
}

/// Multi-page PDFs import at most this many pages (one floor each).
pub const MAX_PDF_PAGES: usize = 40;

/// Reads a floor plan file: one plan per page for a PDF (at most
/// [`MAX_PDF_PAGES`]), otherwise one.
pub fn import_plan_pages(path: &Path) -> Result<Vec<ImportedPlan>, IoError> {
    let is_pdf = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
    if !is_pdf {
        return Ok(vec![import_plan(path)?]);
    }
    let bytes = std::fs::read(path)?;
    let n = crate::pdf::page_count(bytes.clone())?.min(MAX_PDF_PAGES);
    let mut out = Vec::with_capacity(n);
    for page in 0..n {
        let png = crate::pdf::rasterize_page(bytes.clone(), page)?.png;
        let image = PlanImage {
            extension: "png".to_owned(),
            bytes: png,
        };
        let decoded = decode_plan(&image)?;
        out.push(ImportedPlan {
            image,
            decoded,
            source_pdf: Some(bytes.clone()),
            source_page: page,
        });
    }
    Ok(out)
}

/// Reads a floor plan file and checks that it decodes.
pub fn import_plan(path: &Path) -> Result<ImportedPlan, IoError> {
    let mut source_pdf = None;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let plan = match ext.as_str() {
        "png" | "jpg" | "jpeg" => PlanImage {
            extension: if ext == "png" { "png" } else { "jpg" }.to_owned(),
            bytes: std::fs::read(path)?,
        },
        // Only the first page is used; the plan is stored as PNG.
        "pdf" => {
            let bytes = std::fs::read(path)?;
            let png = crate::pdf::rasterize_first_page(bytes.clone())?.png;
            source_pdf = Some(bytes);
            PlanImage {
                extension: "png".to_owned(),
                bytes: png,
            }
        }
        other => {
            return Err(IoError::Unsupported(format!(
                "floor plan format {other:?} (use PNG, JPEG or PDF)"
            )))
        }
    };
    let decoded = decode_plan(&plan)?;
    Ok(ImportedPlan {
        image: plan,
        decoded,
        source_pdf,
        source_page: 0,
    })
}

pub fn decode_plan(plan: &PlanImage) -> Result<DecodedPlan, IoError> {
    let format = match plan.extension.as_str() {
        "png" => image::ImageFormat::Png,
        "jpg" | "jpeg" => image::ImageFormat::Jpeg,
        other => return Err(IoError::Unsupported(format!("image type {other:?}"))),
    };
    let img = image::load_from_memory_with_format(&plan.bytes, format)?;
    if img.width() > MAX_SIDE_PX || img.height() > MAX_SIDE_PX {
        return Err(IoError::Unsupported(format!(
            "image is {}×{} px; the maximum is {MAX_SIDE_PX} px per side",
            img.width(),
            img.height()
        )));
    }
    let rgba = img.into_rgba8();
    Ok(DecodedPlan {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_bytes(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([255, 0, 0, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn decodes_png() {
        let plan = PlanImage {
            extension: "png".into(),
            bytes: png_bytes(30, 20),
        };
        let d = decode_plan(&plan).unwrap();
        assert_eq!((d.width, d.height), (30, 20));
        assert_eq!(d.rgba.len(), 30 * 20 * 4);
        assert_eq!(&d.rgba[..4], &[255, 0, 0, 255]);
    }

    #[test]
    fn imports_pdf_as_png() {
        let dir = std::env::temp_dir().join(format!("ondalux-pdf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("plan.pdf");
        std::fs::write(&path, crate::pdf::tests::sample_pdf()).unwrap();
        let ImportedPlan {
            image: plan,
            decoded,
            ..
        } = import_plan(&path).unwrap();
        assert_eq!(plan.extension, "png");
        assert_eq!(decoded.width, 2 * decoded.height);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unknown_formats_are_rejected() {
        let err = import_plan(Path::new("plan.bmp")).unwrap_err();
        assert!(matches!(err, IoError::Unsupported(_)));
    }

    #[test]
    fn corrupt_image_fails() {
        let plan = PlanImage {
            extension: "png".into(),
            bytes: vec![1, 2, 3],
        };
        assert!(decode_plan(&plan).is_err());
    }

    #[test]
    fn multi_page_pdfs_give_one_plan_per_page() {
        let dir = std::env::temp_dir().join(format!("ondalux-pages-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("plans.pdf");
        std::fs::write(&path, crate::pdf::tests::two_page_pdf()).unwrap();
        let pages = import_plan_pages(&path).unwrap();
        assert_eq!(pages.len(), 2);
        assert_eq!((pages[0].source_page, pages[1].source_page), (0, 1));
        // 2:1 and 3:1 pages, both scaled to the same longest side.
        assert!(pages[1].decoded.height < pages[0].decoded.height);
        assert!(pages.iter().all(|p| p.source_pdf.is_some()));
        // Images give a single plan.
        let png = dir.join("plan.png");
        std::fs::write(&png, png_bytes(40, 20)).unwrap();
        assert_eq!(import_plan_pages(&png).unwrap().len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
