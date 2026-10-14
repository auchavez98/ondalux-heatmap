//! Wall detection from the imported floor plan: builds the ink mask (from
//! the PDF line art when the plan came from a vector PDF, otherwise from
//! the image) and runs `ondalux_core::walldetect`.

use crate::error::IoError;
use crate::plan_image::DecodedPlan;
use ondalux_core::geometry::Vec2;
use ondalux_core::walldetect::{self, DetectParams, DetectedWall, Mask};

/// Detection works on at most this many pixels per side (speed); results
/// are scaled back to plan pixels.
pub const DETECT_MAX_SIDE: usize = 3000;

/// Where the ink mask came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaskSource {
    /// Line art of a vector PDF (no text, images or dashed lines).
    PdfVectors,
    /// The plan image, thresholded.
    Image,
}

/// A prepared mask, so the user can tweak parameters without rebuilding it.
pub struct PlanMask {
    pub mask: Mask,
    /// Plan pixels per mask pixel.
    pub scale: f32,
    pub source: MaskSource,
    /// Parameters estimated from the plan (in mask pixels).
    pub auto: DetectParams,
}

/// Builds the mask: from the PDF when available and it has enough line
/// art, otherwise from the image.
pub fn plan_mask(plan: &DecodedPlan, source_pdf: Option<&[u8]>) -> Result<PlanMask, IoError> {
    let (w, h) = (plan.width as usize, plan.height as usize);
    let factor = w.max(h).div_ceil(DETECT_MAX_SIDE).max(1);
    let (mw, mh) = (w.div_ceil(factor), h.div_ceil(factor));
    if let Some(pdf) = source_pdf {
        if let Ok(mask) = crate::pdf::vector_mask(pdf.to_vec(), mw, mh) {
            // A scanned PDF has (almost) no line art: fall back to the image.
            if mask.ink_count() > mw * mh / 500 {
                let auto = walldetect::estimate_params(&mask);
                return Ok(PlanMask {
                    mask,
                    scale: w as f32 / mw as f32,
                    source: MaskSource::PdfVectors,
                    auto,
                });
            }
        }
    }
    let luma: Vec<u8> = plan
        .rgba
        .chunks_exact(4)
        .map(|p| {
            // Transparent pixels count as white paper.
            let a = p[3] as f32 / 255.0;
            let l = 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32;
            (l * a + 255.0 * (1.0 - a)) as u8
        })
        .collect();
    let threshold = Mask::otsu_threshold(&luma);
    let full = Mask::from_luma(w, h, &luma, threshold);
    let mask = full.downscale(factor);
    let auto = walldetect::estimate_params(&mask);
    Ok(PlanMask {
        scale: w as f32 / mask.width as f32,
        mask,
        source: MaskSource::Image,
        auto,
    })
}

/// User-facing knobs on top of the estimated parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tuning {
    /// 0 (strict: only clear, long, thick walls) … 1 (permissive).
    pub sensitivity: f32,
}

impl Default for Tuning {
    fn default() -> Self {
        Self { sensitivity: 0.5 }
    }
}

impl PlanMask {
    pub fn params(&self, tuning: Tuning) -> DetectParams {
        // sensitivity 0.5 = estimated values; lower is stricter.
        let k = 2f32.powf((0.5 - tuning.sensitivity.clamp(0.0, 1.0)) * 2.0);
        DetectParams {
            min_length_px: self.auto.min_length_px * k,
            min_thickness_px: (self.auto.min_thickness_px * k.sqrt()).max(1.0),
            max_thickness_px: self.auto.max_thickness_px,
            max_gap_px: self.auto.max_gap_px,
        }
    }

    /// Walls in plan pixels.
    pub fn detect(&self, tuning: Tuning) -> Vec<DetectedWall> {
        let s = self.scale;
        walldetect::detect(&self.mask, &self.params(tuning))
            .into_iter()
            .map(|w| DetectedWall {
                a: Vec2::new(w.a.x * s, w.a.y * s),
                b: Vec2::new(w.b.x * s, w.b.y * s),
                thickness_px: w.thickness_px * s,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// White 600×400 image with a black rectangle outline 10 px thick.
    fn boxed_plan() -> DecodedPlan {
        let (w, h) = (600u32, 400u32);
        let mut rgba = vec![255u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let inside_outer = (50..550).contains(&x) && (50..350).contains(&y);
                let inside_inner = (60..540).contains(&x) && (60..340).contains(&y);
                if inside_outer && !inside_inner {
                    let i = ((y * w + x) * 4) as usize;
                    rgba[i..i + 3].fill(0);
                }
            }
        }
        DecodedPlan {
            width: w,
            height: h,
            rgba,
        }
    }

    #[test]
    fn detects_the_four_sides_of_an_image_plan() {
        let pm = plan_mask(&boxed_plan(), None).unwrap();
        assert_eq!(pm.source, MaskSource::Image);
        let walls = pm.detect(Tuning::default());
        assert_eq!(walls.len(), 4, "{walls:?}");
        for w in &walls {
            assert!((w.thickness_px - 10.0).abs() <= 1.0, "{w:?}");
        }
    }

    #[test]
    fn detects_walls_from_pdf_line_art_and_ignores_text() {
        // 300 × 200 pt page: a 6 pt thick stroked rectangle and some text.
        let content = "0 0 0 RG 6 w 30 30 240 140 re S BT /F1 12 Tf 100 100 Td (Room) Tj ET";
        let pdf = crate::pdf::tests::pdf_with_content(content, 300, 200);
        let plan = DecodedPlan {
            width: 1500,
            height: 1000,
            rgba: vec![255; 1500 * 1000 * 4],
        };
        let pm = plan_mask(&plan, Some(&pdf)).unwrap();
        assert_eq!(pm.source, MaskSource::PdfVectors);
        let walls = pm.detect(Tuning::default());
        assert_eq!(walls.len(), 4, "{walls:?}");
        // 6 pt at 5 px/pt = 30 px.
        for w in &walls {
            assert!((w.thickness_px - 30.0).abs() <= 3.0, "{w:?}");
        }
    }
}
