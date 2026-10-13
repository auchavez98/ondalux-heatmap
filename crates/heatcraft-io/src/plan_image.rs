//! Floor plan import (PNG, JPEG).
//!
//! STUB: single-page PDF import is not implemented yet; see the PDF spike
//! planned at the end of Phase 1.

use crate::error::IoError;
use crate::project_file::PlanImage;
use std::path::Path;

/// Largest accepted side, in pixels, to keep memory and GPU textures sane.
pub const MAX_SIDE_PX: u32 = 16_384;

/// Decoded RGBA8 pixels, ready for display.
#[derive(Debug, Clone)]
pub struct DecodedPlan {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Reads a floor plan file and checks that it decodes.
pub fn import_plan(path: &Path) -> Result<(PlanImage, DecodedPlan), IoError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let extension = match ext.as_str() {
        "png" => "png",
        "jpg" | "jpeg" => "jpg",
        "pdf" => {
            return Err(IoError::Unsupported(
                "PDF floor plans are not supported yet; export the page to PNG first".into(),
            ))
        }
        other => {
            return Err(IoError::Unsupported(format!(
                "floor plan format {other:?} (use PNG or JPEG)"
            )))
        }
    };
    let plan = PlanImage {
        extension: extension.to_owned(),
        bytes: std::fs::read(path)?,
    };
    let decoded = decode_plan(&plan)?;
    Ok((plan, decoded))
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
    fn pdf_is_reported_as_unsupported() {
        let err = import_plan(Path::new("plan.pdf")).unwrap_err();
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
}
