//! Canvas viewport: maps image pixels to screen points (pan + zoom).

use eframe::egui::{Pos2, Rect, Vec2};
use ondalux_core::geometry::Vec2 as WVec2;

pub const MIN_ZOOM: f32 = 0.02;
pub const MAX_ZOOM: f32 = 40.0;

#[derive(Debug, Clone, Copy)]
pub struct View {
    /// Screen position of image pixel (0, 0).
    pub origin: Pos2,
    /// Screen points per image pixel.
    pub zoom: f32,
}

impl Default for View {
    fn default() -> Self {
        Self {
            origin: Pos2::ZERO,
            zoom: 1.0,
        }
    }
}

impl View {
    /// Fits an image of `size` pixels inside `rect` with a small margin.
    pub fn fit(rect: Rect, size: Vec2) -> Self {
        let margin = 24.0;
        let avail = (rect.size() - Vec2::splat(2.0 * margin)).max(Vec2::splat(1.0));
        let zoom = (avail.x / size.x)
            .min(avail.y / size.y)
            .clamp(MIN_ZOOM, MAX_ZOOM);
        let origin = rect.center() - size * zoom * 0.5;
        Self { origin, zoom }
    }

    pub fn to_screen(self, px: WVec2) -> Pos2 {
        self.origin + Vec2::new(px.x, px.y) * self.zoom
    }

    pub fn to_image(self, screen: Pos2) -> WVec2 {
        let v = (screen - self.origin) / self.zoom;
        WVec2::new(v.x, v.y)
    }

    /// Zooms by `factor` keeping the image point under `anchor` fixed.
    pub fn zoom_around(&mut self, anchor: Pos2, factor: f32) {
        let new_zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let k = new_zoom / self.zoom;
        self.origin = anchor + (self.origin - anchor) * k;
        self.zoom = new_zoom;
    }

    pub fn image_rect(&self, size: Vec2) -> Rect {
        Rect::from_min_size(self.origin, size * self.zoom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_zoom_anchor() {
        let mut v = View {
            origin: Pos2::new(10.0, 20.0),
            zoom: 2.0,
        };
        let p = WVec2::new(30.0, 40.0);
        let s = v.to_screen(p);
        assert_eq!(s, Pos2::new(70.0, 100.0));
        v.zoom_around(s, 1.5);
        let back = v.to_image(s);
        assert!((back.x - p.x).abs() < 1e-3 && (back.y - p.y).abs() < 1e-3);
    }

    #[test]
    fn fit_centers_image() {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 600.0));
        let v = View::fit(rect, Vec2::new(2000.0, 1000.0));
        let r = v.image_rect(Vec2::new(2000.0, 1000.0));
        assert!((r.center() - rect.center()).length() < 1e-3);
        assert!(r.width() <= 1000.0 && r.height() <= 600.0);
    }
}
