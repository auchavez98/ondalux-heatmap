//! Renders a floor plan with a heat map, walls, APs, coverage gaps and a
//! legend into an RGBA image (used for PNG export and inside PDF reports).

use crate::raster::{Canvas, Color, BLACK, WHITE};
use image::{imageops, Rgba, RgbaImage};
use ondalux_core::analysis::GapZone;
use ondalux_core::channel::Band;
use ondalux_core::colormap::ColorScale;
use ondalux_core::geometry::Vec2;
use ondalux_core::grid::GridSpec;
use ondalux_core::project::Project;
use ondalux_io::DecodedPlan;

/// Values to color over the plan (`NaN` = transparent).
pub struct HeatLayer<'a> {
    pub grid: GridSpec,
    pub values: &'a [f32],
    pub scale: ColorScale,
    /// 0–1.
    pub opacity: f32,
}

/// Coverage gaps to outline and number.
pub struct GapOverlay<'a> {
    pub grid: GridSpec,
    /// Gap number per cell (0 = none).
    pub labels: &'a [u32],
    pub zones: &'a [GapZone],
}

pub struct MapOptions<'a> {
    /// Output width limit; the plan is never upscaled.
    pub max_width_px: u32,
    pub heat: Option<HeatLayer<'a>>,
    pub gaps: Option<GapOverlay<'a>>,
    /// Legend title; no legend when `None` or when there is no heat layer.
    pub legend_title: Option<String>,
    /// Band whose channels are printed next to each AP.
    pub band: Option<Band>,
    pub show_walls: bool,
    pub show_aps: bool,
    /// Multiplies label, AP and legend sizes (e.g. for images shrunk in a PDF).
    pub label_scale: f32,
    /// Replaces the tick labels of a discrete scale (one per stop).
    pub discrete_labels: Option<Vec<String>>,
}

impl Default for MapOptions<'_> {
    fn default() -> Self {
        Self {
            max_width_px: 1600,
            heat: None,
            gaps: None,
            legend_title: None,
            band: None,
            show_walls: true,
            show_aps: true,
            label_scale: 1.0,
            discrete_labels: None,
        }
    }
}

fn sample(layer: &HeatLayer, m: Vec2) -> Option<f32> {
    let g = layer.grid;
    let gx = (m.x - g.origin.x) / g.cell_m - 0.5;
    let gy = (m.y - g.origin.y) / g.cell_m - 0.5;
    let at = |c: i64, r: i64| -> Option<f32> {
        if c < 0 || r < 0 || c as usize >= g.cols || r as usize >= g.rows {
            return None;
        }
        let v = layer.values[r as usize * g.cols + c as usize];
        (!v.is_nan()).then_some(v)
    };
    let nearest = at(gx.round() as i64, gy.round() as i64);
    if layer.scale.is_discrete() {
        return nearest;
    }
    let (c0, r0) = (gx.floor() as i64, gy.floor() as i64);
    let (tx, ty) = (gx - c0 as f32, gy - r0 as f32);
    match (
        at(c0, r0),
        at(c0 + 1, r0),
        at(c0, r0 + 1),
        at(c0 + 1, r0 + 1),
    ) {
        (Some(a), Some(b), Some(c), Some(d)) => {
            let top = a + (b - a) * tx;
            let bottom = c + (d - c) * tx;
            Some(top + (bottom - top) * ty)
        }
        _ => nearest,
    }
}

pub fn render_map(project: &Project, plan: Option<&DecodedPlan>, opts: &MapOptions) -> RgbaImage {
    let (w_px, h_px) = (project.plan.width_px.max(1), project.plan.height_px.max(1));
    let s = (opts.max_width_px as f32 / w_px as f32).min(1.0);
    let (out_w, out_h) = (
        ((w_px as f32 * s).round() as u32).max(1),
        ((h_px as f32 * s).round() as u32).max(1),
    );
    let base = match plan {
        Some(p) if p.width > 0 && p.height > 0 => {
            let src = RgbaImage::from_raw(p.width, p.height, p.rgba.clone())
                .unwrap_or_else(|| RgbaImage::from_pixel(p.width, p.height, Rgba(WHITE)));
            let mut img = imageops::resize(&src, out_w, out_h, imageops::FilterType::Triangle);
            // Flatten transparent plans onto white.
            for p in img.pixels_mut() {
                let a = p.0[3] as f32 / 255.0;
                for k in 0..3 {
                    p.0[k] = (p.0[k] as f32 * a + 255.0 * (1.0 - a)).round() as u8;
                }
                p.0[3] = 255;
            }
            img
        }
        _ => RgbaImage::from_pixel(out_w, out_h, Rgba([245, 245, 242, 255])),
    };
    let mut cv = Canvas::new(base);
    // Output pixels per meter.
    let ppm = project.plan.px_per_m() * s;
    let to_px = |m: Vec2| (m.x * ppm, m.y * ppm);
    let ui = (out_w as f32 / 1200.0).clamp(0.6, 2.0) * opts.label_scale.max(0.1);

    if let Some(layer) = &opts.heat {
        let alpha = (layer.opacity.clamp(0.0, 1.0) * 255.0) as u8;
        for py in 0..out_h {
            for px in 0..out_w {
                let m = Vec2::new((px as f32 + 0.5) / ppm, (py as f32 + 0.5) / ppm);
                if let Some(v) = sample(layer, m) {
                    let [r, g, b] = layer.scale.rgb(v);
                    cv.blend(px as i32, py as i32, [r, g, b, alpha], 1.0);
                }
            }
        }
    }

    if let Some(gaps) = &opts.gaps {
        let g = gaps.grid;
        for (i, &l) in gaps.labels.iter().enumerate() {
            if l == 0 {
                continue;
            }
            let p = g.sample(i % g.cols, i / g.cols);
            let (x, y) = to_px(Vec2::new(p.x - g.cell_m / 2.0, p.y - g.cell_m / 2.0));
            let size = g.cell_m * ppm;
            cv.fill_rect(x, y, size, size, [200, 30, 30, 70]);
        }
        for z in gaps.zones {
            let (x0, y0) = to_px(z.min);
            let (x1, y1) = to_px(z.max);
            cv.stroke_rect(x0, y0, x1 - x0, y1 - y0, 1.5 * ui, [180, 20, 20, 230]);
            let (cx, cy) = to_px(z.centroid);
            let r = 11.0 * ui;
            cv.fill_circle(cx, cy, r, [180, 20, 20, 240]);
            let text = z.number.to_string();
            let fs = 13.0 * ui;
            let (tw, th) = cv.text_size(&text, fs);
            cv.text(cx - tw / 2.0, cy - th / 2.0, &text, fs, WHITE);
        }
    }

    if opts.show_walls {
        let width = (0.12 * ppm).clamp(1.5, 6.0);
        for w in &project.walls {
            let c = project
                .materials
                .get(w.material)
                .map_or([90, 90, 90, 255], |m| m.color);
            let (ax, ay) = to_px(w.segment.a);
            let (bx, by) = to_px(w.segment.b);
            cv.line(ax, ay, bx, by, width, darken(c));
        }
    }

    if opts.show_aps {
        for ap in &project.aps {
            let (x, y) = to_px(ap.position);
            let has_band = opts
                .band
                .is_none_or(|b| ap.enabled_radios().any(|(_, r)| r.channel.band == b));
            let fill: Color = if has_band {
                [40, 110, 220, 255]
            } else {
                [130, 130, 130, 255]
            };
            cv.fill_circle(x, y, 8.0 * ui, WHITE);
            cv.fill_circle(x, y, 6.0 * ui, fill);
            let channels: Vec<String> = ap
                .enabled_radios()
                .filter(|(_, r)| opts.band.is_none_or(|b| r.channel.band == b))
                .map(|(_, r)| r.channel.number.to_string())
                .collect();
            let label = if channels.is_empty() || opts.band.is_none() {
                ap.name.clone()
            } else {
                format!("{} · ch {}", ap.name, channels.join("/"))
            };
            let fs = 12.0 * ui;
            let (tw, _) = cv.text_size(&label, fs);
            cv.label(x - tw / 2.0, y + 11.0 * ui, &label, fs);
        }
    }

    if let (Some(layer), Some(title)) = (&opts.heat, &opts.legend_title) {
        // The legend goes in a strip below the plan so it never hides it.
        let strip = (76.0 * ui).ceil() as u32;
        let mut tall = RgbaImage::from_pixel(out_w, out_h + strip, Rgba(WHITE));
        imageops::overlay(&mut tall, &cv.img, 0, 0);
        let mut cv = Canvas::new(tall);
        draw_legend(
            &mut cv,
            layer.scale,
            title,
            ui,
            opts.discrete_labels.as_deref(),
        );
        return cv.img;
    }
    cv.img
}

fn darken(c: [u8; 4]) -> Color {
    [
        (c[0] as f32 * 0.55) as u8,
        (c[1] as f32 * 0.55) as u8,
        (c[2] as f32 * 0.55) as u8,
        255,
    ]
}

fn draw_legend(
    cv: &mut Canvas,
    scale: ColorScale,
    title: &str,
    ui: f32,
    labels: Option<&[String]>,
) {
    let fs = 12.0 * ui;
    let small = 11.0 * ui;
    let pad = 8.0 * ui;
    let width = (300.0 * ui).max(cv.text_size(title, fs).0 + 2.0 * pad);
    let height = 60.0 * ui;
    let x = 12.0 * ui;
    let y = cv.height() as f32 - height - 8.0 * ui;
    cv.fill_rect(x, y, width, height, [255, 255, 255, 235]);
    cv.stroke_rect(x, y, width, height, 1.0, [120, 120, 120, 255]);
    cv.text(x + pad, y + pad * 0.6, title, fs, BLACK);
    let bar_y = y + 26.0 * ui;
    let bar_h = 12.0 * ui;
    let bar_w = width - 2.0 * pad;
    let stops = scale.stops();
    let (lo, hi) = (stops[0].0, stops[stops.len() - 1].0);
    if scale.is_discrete() {
        let n = stops.len();
        for (i, (v, _)) in stops.iter().enumerate() {
            let cx0 = x + pad + bar_w * i as f32 / n as f32;
            let cw = bar_w / n as f32;
            let [r, g, b] = scale.rgb(*v);
            cv.fill_rect(cx0, bar_y, cw, bar_h, [r, g, b, 255]);
            let t = if let Some(l) = labels.and_then(|l| l.get(i)) {
                l.clone()
            } else if i == n - 1 && n > 2 {
                format!("{v:.0}+")
            } else {
                format!("{v:.0}")
            };
            let (tw, _) = cv.text_size(&t, small);
            cv.text(
                cx0 + cw / 2.0 - tw / 2.0,
                bar_y + bar_h + 2.0,
                &t,
                small,
                BLACK,
            );
        }
    } else {
        let steps = bar_w.max(1.0) as i32;
        for k in 0..steps {
            let v = lo + (hi - lo) * (k as f32 + 0.5) / steps as f32;
            let [r, g, b] = scale.rgb(v);
            cv.fill_rect(x + pad + k as f32, bar_y, 1.0, bar_h, [r, g, b, 255]);
        }
        for (v, _) in stops {
            let tx = x + pad + bar_w * (v - lo) / (hi - lo);
            let t = if scale == ColorScale::Difference && *v > 0.0 {
                format!("+{v:.0}")
            } else {
                format!("{v:.0}")
            };
            let (tw, _) = cv.text_size(&t, small);
            cv.text(tx - tw / 2.0, bar_y + bar_h + 2.0, &t, small, BLACK);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ondalux_core::metrics::MetricKind;

    #[test]
    fn renders_heat_walls_and_legend_at_plan_scale() {
        let project = Project::new("t", 400, 200); // 20 × 10 m at 20 px/m
        let grid = GridSpec::covering(20.0, 10.0, 1.0);
        // Left half strong, right half no data.
        let values: Vec<f32> = (0..grid.len())
            .map(|i| if i % grid.cols < 10 { -50.0 } else { f32::NAN })
            .collect();
        let img = render_map(
            &project,
            None,
            &MapOptions {
                heat: Some(HeatLayer {
                    grid,
                    values: &values,
                    scale: ColorScale::Metric(MetricKind::Rssi),
                    opacity: 1.0,
                }),
                legend_title: Some("Signal (dBm)".into()),
                ..Default::default()
            },
        );
        assert_eq!(img.width(), 400);
        assert!(img.height() > 200, "legend strip below the plan");
        let strong = ColorScale::Metric(MetricKind::Rssi).rgb(-50.0);
        assert_eq!(&img.get_pixel(100, 20).0[..3], &strong);
        // No data: background stays.
        assert_eq!(img.get_pixel(390, 20).0, [245, 245, 242, 255]);
    }

    #[test]
    fn large_plans_are_downscaled() {
        let project = Project::new("t", 4000, 1000);
        let img = render_map(&project, None, &MapOptions::default());
        assert_eq!((img.width(), img.height()), (1600, 400));
    }
}
