//! Design analysis for reports: coverage gaps and rule-based
//! recommendations. Every rule is listed in `docs/reports.md`.

use crate::ap::AccessPoint;
use crate::channel::{Band, ChannelWidth};
use crate::geometry::Vec2;
use crate::grid::GridSpec;
use crate::metrics::{Metrics, NO_SERVER};
use crate::predict::{rssi_at, Obstacles};
use crate::project::Project;
use crate::units::Dbm;

/// Coverage at or above this share of the analysed area is considered good.
pub const TARGET_COVERAGE: f32 = 0.95;
/// Gaps smaller than this are ignored (noise at wall corners).
pub const MIN_GAP_AREA_M2: f32 = 1.0;
/// Share of covered cells hearing ≥ 2 overlapping radios above CCA that
/// triggers the co-channel warning.
pub const OVERLAP_WARNING_SHARE: f32 = 0.2;

/// A connected area that does not meet the coverage requirements.
#[derive(Debug, Clone, PartialEq)]
pub struct GapZone {
    /// 1-based, in decreasing area order.
    pub number: usize,
    pub area_m2: f32,
    pub centroid: Vec2,
    pub min: Vec2,
    pub max: Vec2,
    /// Strongest signal anywhere in the zone (`None` = no signal at all).
    pub best_rssi: Option<Dbm>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Severity {
    Critical,
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Finding {
    NoAccessPoints,
    /// Coverage below [`TARGET_COVERAGE`].
    LowCoverage {
        ratio: f32,
    },
    /// Suggest an AP near a large gap.
    AddApNearGap {
        zone: usize,
        at: Vec2,
        area_m2: f32,
    },
    /// Many places hear ≥ 2 radios on overlapping channels.
    ChannelOverlap {
        share: f32,
    },
    /// Two radios on overlapping channels hear each other above CCA.
    CoChannelPair {
        a: String,
        b: String,
        channel: u16,
        rssi: Dbm,
    },
    /// 2.4 GHz radios outside the non-overlapping 1/6/11 plan.
    NonStandard24Channels {
        aps: Vec<String>,
    },
    /// 2.4 GHz radios using 40 MHz.
    Wide24Channels {
        aps: Vec<String>,
    },
    /// Wide channels in a dense design where overlap is high.
    ReduceChannelWidth {
        width: ChannelWidth,
        aps: Vec<String>,
    },
    /// Survey and prediction disagree systematically.
    ModelBias {
        mean_db: f32,
        rmse_db: f32,
        points: usize,
    },
    /// Coverage meets the target but some small areas do not.
    MinorGaps {
        count: usize,
        area_m2: f32,
    },
    /// Everything meets the requirements.
    AllGood,
}

impl Finding {
    pub fn severity(&self) -> Severity {
        match self {
            Finding::NoAccessPoints => Severity::Critical,
            Finding::LowCoverage { ratio } if *ratio < 0.8 => Severity::Critical,
            Finding::LowCoverage { .. }
            | Finding::AddApNearGap { .. }
            | Finding::ChannelOverlap { .. }
            | Finding::CoChannelPair { .. }
            | Finding::ModelBias { .. } => Severity::Warning,
            Finding::NonStandard24Channels { .. }
            | Finding::Wide24Channels { .. }
            | Finding::ReduceChannelWidth { .. }
            | Finding::MinorGaps { .. }
            | Finding::AllGood => Severity::Info,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BandAnalysis {
    pub band: Band,
    /// Cells analysed (inside the building footprint).
    pub analysed_cells: usize,
    pub coverage_ratio: f32,
    /// Share of covered cells hearing ≥ 2 overlapping radios above CCA.
    pub overlap_share: f32,
    pub gaps: Vec<GapZone>,
    /// Gap number per cell (0 = no gap / not analysed), row-major.
    pub gap_labels: Vec<u32>,
    pub findings: Vec<Finding>,
}

/// Cells to analyse: inside the bounding box of the walls (the building
/// footprint), or the whole plan if there are no walls.
pub fn footprint_mask(project: &Project, grid: &GridSpec) -> Vec<bool> {
    let mut lo = Vec2::new(f32::MAX, f32::MAX);
    let mut hi = Vec2::new(f32::MIN, f32::MIN);
    for w in &project.walls {
        lo = lo.min(w.segment.a).min(w.segment.b);
        hi = hi.max(w.segment.a).max(w.segment.b);
    }
    let has_walls = !project.walls.is_empty();
    (0..grid.len())
        .map(|i| {
            if !has_walls {
                return true;
            }
            let p = grid.sample(i % grid.cols, i / grid.cols);
            p.x >= lo.x && p.x <= hi.x && p.y >= lo.y && p.y <= hi.y
        })
        .collect()
}

/// Labels 4-connected regions of cells where `bad` is true.
fn connected_gaps(metrics: &Metrics, bad: &[bool]) -> (Vec<GapZone>, Vec<u32>) {
    let g = metrics.grid;
    let cell_area = g.cell_m * g.cell_m;
    let mut label = vec![0u32; g.len()];
    let mut zones: Vec<(Vec<usize>, GapZone)> = Vec::new();
    let mut stack = Vec::new();
    for start in 0..g.len() {
        if !bad[start] || label[start] != 0 {
            continue;
        }
        let id = zones.len() as u32 + 1;
        label[start] = id;
        stack.push(start);
        let mut cells = Vec::new();
        while let Some(i) = stack.pop() {
            cells.push(i);
            let (c, r) = (i % g.cols, i / g.cols);
            let mut visit = |j: usize| {
                if bad[j] && label[j] == 0 {
                    label[j] = id;
                    stack.push(j);
                }
            };
            if c > 0 {
                visit(i - 1);
            }
            if c + 1 < g.cols {
                visit(i + 1);
            }
            if r > 0 {
                visit(i - g.cols);
            }
            if r + 1 < g.rows {
                visit(i + g.cols);
            }
        }
        let mut sum = Vec2::default();
        let mut lo = Vec2::new(f32::MAX, f32::MAX);
        let mut hi = Vec2::new(f32::MIN, f32::MIN);
        let mut best: Option<f32> = None;
        for &i in &cells {
            let p = g.sample(i % g.cols, i / g.cols);
            sum = sum + p;
            lo = lo.min(p);
            hi = hi.max(p);
            if metrics.serving[i] != NO_SERVER {
                best = Some(best.map_or(metrics.rssi[i], |b: f32| b.max(metrics.rssi[i])));
            }
        }
        let n = cells.len() as f32;
        let half = Vec2::new(g.cell_m / 2.0, g.cell_m / 2.0);
        zones.push((
            cells,
            GapZone {
                number: 0,
                area_m2: n * cell_area,
                centroid: sum * (1.0 / n),
                min: lo - half,
                max: hi + half,
                best_rssi: best.map(Dbm),
            },
        ));
    }
    // Keep zones above the minimum area, numbered by decreasing area.
    let mut order: Vec<usize> = (0..zones.len())
        .filter(|&z| zones[z].1.area_m2 >= MIN_GAP_AREA_M2)
        .collect();
    order.sort_by(|&a, &b| zones[b].1.area_m2.total_cmp(&zones[a].1.area_m2));
    let mut out_labels = vec![0u32; g.len()];
    let mut out = Vec::with_capacity(order.len());
    for (rank, &z) in order.iter().enumerate() {
        let (cells, zone) = &zones[z];
        for &i in cells {
            out_labels[i] = rank as u32 + 1;
        }
        let mut zone = zone.clone();
        zone.number = rank + 1;
        out.push(zone);
    }
    (out, out_labels)
}

fn ap_names(aps: &[&AccessPoint]) -> Vec<String> {
    let mut v: Vec<String> = aps.iter().map(|a| a.name.clone()).collect();
    v.sort();
    v.dedup();
    v
}

/// Analyses one band of a prediction (or of interpolated measurements).
pub fn analyze_band(
    project: &Project,
    band: Band,
    metrics: &Metrics,
    obstacles: &Obstacles,
) -> BandAnalysis {
    let g = metrics.grid;
    let inside = footprint_mask(project, &g);
    let analysed: Vec<usize> = (0..g.len()).filter(|&i| inside[i]).collect();
    let n = analysed.len().max(1) as f32;
    let covered = analysed
        .iter()
        .filter(|&&i| metrics.meets_requirements(i))
        .count();
    let coverage_ratio = covered as f32 / n;
    let served: Vec<usize> = analysed
        .iter()
        .copied()
        .filter(|&i| {
            metrics.serving[i] != NO_SERVER && metrics.rssi[i] >= metrics.requirements.min_rssi.0
        })
        .collect();
    let overlap_share = if served.is_empty() {
        0.0
    } else {
        served.iter().filter(|&&i| metrics.overlap[i] >= 2).count() as f32 / served.len() as f32
    };
    let bad: Vec<bool> = (0..g.len())
        .map(|i| inside[i] && !metrics.meets_requirements(i))
        .collect();
    let (gaps, gap_labels) = connected_gaps(metrics, &bad);

    let mut findings = Vec::new();
    let radios: Vec<(&AccessPoint, &crate::ap::Radio)> = project
        .aps
        .iter()
        .flat_map(|a| a.enabled_radios().map(move |(_, r)| (a, r)))
        .filter(|(_, r)| r.channel.band == band)
        .collect();
    if radios.is_empty() {
        findings.push(Finding::NoAccessPoints);
    } else {
        if coverage_ratio >= TARGET_COVERAGE && !gaps.is_empty() {
            findings.push(Finding::MinorGaps {
                count: gaps.len(),
                area_m2: gaps.iter().map(|z| z.area_m2).sum(),
            });
        }
        if coverage_ratio < TARGET_COVERAGE {
            findings.push(Finding::LowCoverage {
                ratio: coverage_ratio,
            });
            for z in gaps.iter().take(3) {
                findings.push(Finding::AddApNearGap {
                    zone: z.number,
                    at: z.centroid,
                    area_m2: z.area_m2,
                });
            }
        }
        if overlap_share > OVERLAP_WARNING_SHARE {
            findings.push(Finding::ChannelOverlap {
                share: overlap_share,
            });
        }
        // Radio pairs on overlapping channels that hear each other.
        let mut scratch = obstacles.scratch();
        for (i, (ap_a, ra)) in radios.iter().enumerate() {
            for (ap_b, rb) in radios.iter().skip(i + 1) {
                if ap_a.id == ap_b.id || ra.channel.range().overlap_mhz(&rb.channel.range()) <= 0.0
                {
                    continue;
                }
                let rssi = rssi_at(
                    ap_a,
                    ra,
                    ap_b.position,
                    obstacles,
                    &project.settings,
                    &mut scratch,
                );
                if rssi.0 >= metrics.requirements.cca_threshold.0 {
                    findings.push(Finding::CoChannelPair {
                        a: ap_a.name.clone(),
                        b: ap_b.name.clone(),
                        channel: ra.channel.number,
                        rssi,
                    });
                }
            }
        }
        if band == Band::G2_4 {
            let odd: Vec<&AccessPoint> = radios
                .iter()
                .filter(|(_, r)| ![1, 6, 11].contains(&r.channel.number))
                .map(|(a, _)| *a)
                .collect();
            if !odd.is_empty() {
                findings.push(Finding::NonStandard24Channels {
                    aps: ap_names(&odd),
                });
            }
            let wide: Vec<&AccessPoint> = radios
                .iter()
                .filter(|(_, r)| r.channel.width > ChannelWidth::Mhz20)
                .map(|(a, _)| *a)
                .collect();
            if !wide.is_empty() {
                findings.push(Finding::Wide24Channels {
                    aps: ap_names(&wide),
                });
            }
        } else if overlap_share > OVERLAP_WARNING_SHARE && radios.len() >= 4 {
            let widest = radios.iter().map(|(_, r)| r.channel.width).max();
            if let Some(w) = widest.filter(|w| *w >= ChannelWidth::Mhz80) {
                let aps: Vec<&AccessPoint> = radios
                    .iter()
                    .filter(|(_, r)| r.channel.width == w)
                    .map(|(a, _)| *a)
                    .collect();
                findings.push(Finding::ReduceChannelWidth {
                    width: w,
                    aps: ap_names(&aps),
                });
            }
        }
        if findings.is_empty() {
            findings.push(Finding::AllGood);
        }
    }

    BandAnalysis {
        band,
        analysed_cells: analysed.len(),
        coverage_ratio,
        overlap_share,
        gaps,
        gap_labels,
        findings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ap::{ApId, Radio};
    use crate::channel::Channel;
    use crate::geometry::Segment;
    use crate::grid::LayerCache;
    use crate::material::{MaterialId, Wall};
    use crate::units::Db;

    fn ap(id: u32, x: f32, y: f32, band: Band, ch: u16, width: ChannelWidth) -> AccessPoint {
        AccessPoint {
            id: ApId(id),
            name: format!("AP-{id}"),
            vendor: String::new(),
            model: String::new(),
            position: Vec2::new(x, y),
            height_m: 3.0,
            orientation_deg: 0.0,
            mount: Default::default(),
            pattern: Default::default(),
            radios: vec![Radio {
                channel: Channel::new(band, ch, width).unwrap(),
                tx_power: Dbm(17.0),
                antenna_gain: Db(4.0),
                enabled: true,
            }],
        }
    }

    fn analyse(project: &Project, band: Band) -> BandAnalysis {
        let (w, h) = project.plan.size_m();
        let grid = GridSpec::covering(w, h, 0.5);
        let obstacles = Obstacles::new(&project.walls, &project.materials);
        let mut cache = LayerCache::new(grid);
        cache.sync(&project.aps, &obstacles, &project.settings);
        let m = Metrics::compute(
            grid,
            &cache.layers(),
            Some(band),
            &project.settings,
            project.requirements,
        );
        analyze_band(project, band, &m, &obstacles)
    }

    fn building(w_px: u32, h_px: u32) -> Project {
        // 20 px/m by default.
        let mut p = Project::new("t", w_px, h_px);
        let (w, h) = p.plan.size_m();
        let c = [
            Vec2::new(0.0, 0.0),
            Vec2::new(w, 0.0),
            Vec2::new(w, h),
            Vec2::new(0.0, h),
        ];
        for k in 0..4 {
            p.walls.push(Wall {
                segment: Segment::new(c[k], c[(k + 1) % 4]),
                material: MaterialId(1),
            });
        }
        p
    }

    #[test]
    fn small_room_with_one_ap_is_fully_covered() {
        let mut p = building(200, 200); // 10 × 10 m
        p.aps
            .push(ap(1, 5.0, 5.0, Band::G5, 36, ChannelWidth::Mhz40));
        let a = analyse(&p, Band::G5);
        assert!(a.coverage_ratio > 0.99, "{}", a.coverage_ratio);
        assert!(a.gaps.is_empty());
        assert_eq!(a.findings, vec![Finding::AllGood]);
    }

    #[test]
    fn concrete_wall_creates_a_gap_with_an_ap_suggestion() {
        let mut p = building(1200, 200); // 60 × 10 m
                                         // A heavy concrete+metal partition splits the building.
        for x in [20.0, 21.0, 22.0] {
            p.walls.push(Wall {
                segment: Segment::new(Vec2::new(x, 0.0), Vec2::new(x, 10.0)),
                material: MaterialId(6),
            });
        }
        p.aps
            .push(ap(1, 5.0, 5.0, Band::G5, 36, ChannelWidth::Mhz40));
        let a = analyse(&p, Band::G5);
        assert!(a.coverage_ratio < TARGET_COVERAGE);
        let gap = &a.gaps[0];
        assert_eq!(gap.number, 1);
        assert!(
            gap.centroid.x > 22.0,
            "gap should be behind the partition: {gap:?}"
        );
        assert!(a
            .findings
            .iter()
            .any(|f| matches!(f, Finding::LowCoverage { .. })));
        let suggestion = a
            .findings
            .iter()
            .find_map(|f| match f {
                Finding::AddApNearGap { at, .. } => Some(*at),
                _ => None,
            })
            .expect("AP suggestion");
        assert!(suggestion.x > 22.0);
        // Labels point at the numbered zones.
        assert!(a.gap_labels.contains(&1));
    }

    #[test]
    fn channel_plan_findings_for_24ghz() {
        let mut p = building(400, 200); // 20 × 10 m
        p.aps
            .push(ap(1, 3.0, 5.0, Band::G2_4, 3, ChannelWidth::Mhz20));
        p.aps
            .push(ap(2, 17.0, 5.0, Band::G2_4, 1, ChannelWidth::Mhz40));
        let a = analyse(&p, Band::G2_4);
        assert!(a.findings.contains(&Finding::NonStandard24Channels {
            aps: vec!["AP-1".into()]
        }));
        assert!(a.findings.contains(&Finding::Wide24Channels {
            aps: vec!["AP-2".into()]
        }));
        // Channels 3 and 1(+5 at 40 MHz) overlap and the APs hear each other.
        assert!(a.findings.iter().any(
            |f| matches!(f, Finding::CoChannelPair { a, b, .. } if a == "AP-1" && b == "AP-2")
        ));
    }

    #[test]
    fn band_without_radios_is_reported() {
        let mut p = building(200, 200);
        p.aps
            .push(ap(1, 5.0, 5.0, Band::G5, 36, ChannelWidth::Mhz20));
        let a = analyse(&p, Band::G6);
        assert_eq!(a.findings, vec![Finding::NoAccessPoints]);
    }

    #[test]
    fn footprint_ignores_area_outside_walls() {
        let mut p = Project::new("t", 400, 400); // 20 × 20 m plan
        p.walls.push(Wall {
            segment: Segment::new(Vec2::new(0.0, 0.0), Vec2::new(10.0, 10.0)),
            material: MaterialId(1),
        });
        let grid = GridSpec::covering(20.0, 20.0, 1.0);
        let mask = footprint_mask(&p, &grid);
        assert_eq!(mask.iter().filter(|&&b| b).count(), 100);
    }
}
