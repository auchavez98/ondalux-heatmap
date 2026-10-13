//! Heat map color scales, shared by the UI and the report renderer so both
//! show exactly the same colors.

use crate::metrics::MetricKind;

/// What a set of values represents, which decides its colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorScale {
    Metric(MetricKind),
    /// Measured − predicted, in dB.
    Difference,
}

/// Color stops `(value, rgb)`, ascending by value.
pub type Stops = &'static [(f32, [u8; 3])];

const RED: [u8; 3] = [215, 48, 39];
const ORANGE: [u8; 3] = [244, 140, 50];
const YELLOW: [u8; 3] = [240, 220, 70];
const LIME: [u8; 3] = [160, 210, 80];
const GREEN: [u8; 3] = [50, 170, 80];
const DARK_GREEN: [u8; 3] = [20, 110, 60];
const TEAL: [u8; 3] = [40, 160, 170];
const BLUE: [u8; 3] = [60, 100, 200];
const NEUTRAL: [u8; 3] = [215, 215, 215];
const LIGHT_BLUE: [u8; 3] = [120, 175, 235];

const RSSI_STOPS: Stops = &[
    (-90.0, RED),
    (-80.0, ORANGE),
    (-72.0, YELLOW),
    (-67.0, LIME),
    (-60.0, GREEN),
    (-45.0, DARK_GREEN),
];
const SNR_STOPS: Stops = &[
    (5.0, RED),
    (12.0, ORANGE),
    (20.0, YELLOW),
    (25.0, LIME),
    (32.0, GREEN),
    (45.0, DARK_GREEN),
];
const COUNT_STOPS: Stops = &[
    (0.0, RED),
    (1.0, YELLOW),
    (2.0, GREEN),
    (3.0, TEAL),
    (4.0, BLUE),
];
const OVERLAP_STOPS: Stops = &[(0.0, GREEN), (1.0, YELLOW), (2.0, ORANGE), (3.0, RED)];
const COVERAGE_STOPS: Stops = &[(0.0, RED), (1.0, GREEN)];
/// Diverging: red = weaker than predicted, blue = stronger.
const DIFF_STOPS: Stops = &[
    (-15.0, RED),
    (-6.0, ORANGE),
    (0.0, NEUTRAL),
    (6.0, LIGHT_BLUE),
    (15.0, BLUE),
];

/// Signal below this is drawn transparent (no usable signal).
pub const RSSI_CUTOFF: f32 = -95.0;

impl ColorScale {
    pub fn stops(self) -> Stops {
        match self {
            ColorScale::Metric(MetricKind::Rssi) => RSSI_STOPS,
            ColorScale::Metric(MetricKind::Snr | MetricKind::Sinr) => SNR_STOPS,
            ColorScale::Metric(MetricKind::ApCount) => COUNT_STOPS,
            ColorScale::Metric(MetricKind::ChannelOverlap) => OVERLAP_STOPS,
            ColorScale::Metric(MetricKind::Coverage) => COVERAGE_STOPS,
            ColorScale::Difference => DIFF_STOPS,
        }
    }

    /// Discrete scales are drawn with hard color steps.
    pub fn is_discrete(self) -> bool {
        matches!(
            self,
            ColorScale::Metric(
                MetricKind::ApCount | MetricKind::ChannelOverlap | MetricKind::Coverage
            )
        )
    }

    pub fn rgb(self, v: f32) -> [u8; 3] {
        let s = self.stops();
        if self.is_discrete() {
            return s
                .iter()
                .rev()
                .find(|(t, _)| v >= *t - 0.5)
                .map_or(s[0].1, |(_, c)| *c);
        }
        if v <= s[0].0 {
            return s[0].1;
        }
        if v >= s[s.len() - 1].0 {
            return s[s.len() - 1].1;
        }
        let i = s.windows(2).position(|w| v < w[1].0).unwrap_or(0);
        let ((a, ca), (b, cb)) = (s[i], s[i + 1]);
        let t = (v - a) / (b - a);
        let lerp = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
        [lerp(ca[0], cb[0]), lerp(ca[1], cb[1]), lerp(ca[2], cb[2])]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RSSI: ColorScale = ColorScale::Metric(MetricKind::Rssi);

    #[test]
    fn continuous_scale_interpolates_and_clamps() {
        assert_eq!(RSSI.rgb(-200.0), RED);
        assert_eq!(RSSI.rgb(-20.0), DARK_GREEN);
        let mid = RSSI.rgb(-85.0);
        assert!(mid[0] > 215 && mid[0] < 244);
    }

    #[test]
    fn discrete_scale_steps() {
        let count = ColorScale::Metric(MetricKind::ApCount);
        assert_eq!(count.rgb(0.0), RED);
        assert_eq!(count.rgb(2.0), GREEN);
        assert_eq!(count.rgb(9.0), BLUE);
        assert_eq!(ColorScale::Metric(MetricKind::Coverage).rgb(1.0), GREEN);
    }

    #[test]
    fn difference_scale_is_diverging() {
        assert_eq!(ColorScale::Difference.rgb(0.0), NEUTRAL);
        assert_eq!(ColorScale::Difference.rgb(-30.0), RED);
        assert_eq!(ColorScale::Difference.rgb(30.0), BLUE);
    }
}
