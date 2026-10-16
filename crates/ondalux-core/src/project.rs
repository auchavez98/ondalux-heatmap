//! The project data model: floors (plan calibration, walls, APs, surveys),
//! materials and prediction settings. Serialized by `ondalux-io`.
//!
//! A project is a building with one or more floors. The floor being edited
//! (the *active* floor) lives in the top-level fields (`plan`, `walls`,
//! `aps`, `surveys`, `floor`), so all per-floor code works on it
//! directly; the other floors wait in `other_floors`. On disk the floors
//! are stored as one ordered list (see `ProjectRepr`).

use crate::ap::{AccessPoint, ApId};
use crate::geometry::Vec2;
use crate::material::{MaterialId, MaterialLibrary, Wall};
use crate::measured::MeasuredSettings;
use crate::metrics::Requirements;
use crate::propagation::PropagationSettings;
use crate::survey::Survey;
use crate::units::Db;
use serde::{Deserialize, Serialize};

/// Scale used until the user calibrates the plan.
pub const DEFAULT_PX_PER_M: f32 = 20.0;

/// Two points on the image (in pixels) and the real distance between them.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Calibration {
    pub a_px: Vec2,
    pub b_px: Vec2,
    pub distance_m: f32,
}

impl Calibration {
    /// Returns `None` if the points coincide or the distance is not positive.
    pub fn new(a_px: Vec2, b_px: Vec2, distance_m: f32) -> Option<Self> {
        let valid = distance_m > 0.0 && distance_m.is_finite() && a_px.distance(b_px) > 1.0;
        valid.then_some(Self {
            a_px,
            b_px,
            distance_m,
        })
    }

    pub fn px_per_m(&self) -> f32 {
        self.a_px.distance(self.b_px) / self.distance_m
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FloorPlan {
    pub width_px: u32,
    pub height_px: u32,
    pub calibration: Option<Calibration>,
}

impl FloorPlan {
    pub fn px_per_m(&self) -> f32 {
        self.calibration.map_or(DEFAULT_PX_PER_M, |c| c.px_per_m())
    }

    pub fn size_m(&self) -> (f32, f32) {
        let s = self.px_per_m();
        (self.width_px as f32 / s, self.height_px as f32 / s)
    }

    pub fn px_to_m(&self, px: Vec2) -> Vec2 {
        px * (1.0 / self.px_per_m())
    }

    pub fn m_to_px(&self, m: Vec2) -> Vec2 {
        m * self.px_per_m()
    }
}

/// Settings of one floor of the building.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FloorInfo {
    /// Display name; empty = numbered by the app.
    pub name: String,
    /// Floor-to-floor height in meters (also the wall height in 3D).
    pub height_m: f32,
    /// Attenuation of the slab under this floor, in dB (all bands).
    pub slab_loss_db: f32,
    /// Where this floor's plan origin sits relative to the building
    /// origin, in meters, so the plans of different floors line up.
    pub offset_m: Vec2,
}

/// Floor-to-floor height used for new floors.
pub const DEFAULT_FLOOR_HEIGHT_M: f32 = 3.0;
/// Typical reinforced concrete slab.
pub const DEFAULT_SLAB_LOSS_DB: f32 = 15.0;
/// APs more than this many floors away are ignored (each slab adds
/// `slab_loss_db`, so they rarely matter).
pub const MAX_FLOORS_AWAY: usize = 2;

impl Default for FloorInfo {
    fn default() -> Self {
        Self {
            name: String::new(),
            height_m: DEFAULT_FLOOR_HEIGHT_M,
            slab_loss_db: DEFAULT_SLAB_LOSS_DB,
            offset_m: Vec2::default(),
        }
    }
}

impl FloorInfo {
    /// The name, or "#n" (1-based) when it has none.
    pub fn label(&self, index: usize) -> String {
        let n = self.name.trim();
        if n.is_empty() {
            format!("#{}", index + 1)
        } else {
            n.to_owned()
        }
    }
}

/// One floor of the building (the stored form of a floor).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Floor {
    #[serde(default)]
    pub info: FloorInfo,
    pub plan: FloorPlan,
    #[serde(default)]
    pub walls: Vec<Wall>,
    #[serde(default)]
    pub aps: Vec<AccessPoint>,
    #[serde(default)]
    pub surveys: Vec<Survey>,
}

impl Floor {
    pub fn new(info: FloorInfo, width_px: u32, height_px: u32) -> Self {
        Self {
            info,
            plan: FloorPlan {
                width_px,
                height_px,
                calibration: None,
            },
            walls: Vec::new(),
            aps: Vec::new(),
            surveys: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "ProjectRepr", into = "ProjectRepr")]
pub struct Project {
    pub name: String,
    /// Plan of the active floor.
    pub plan: FloorPlan,
    pub materials: MaterialLibrary,
    pub walls: Vec<Wall>,
    pub aps: Vec<AccessPoint>,
    pub settings: PropagationSettings,
    pub requirements: Requirements,
    /// Heat map sampling step, in meters.
    pub grid_cell_m: f32,
    /// Field surveys of the active floor.
    pub surveys: Vec<Survey>,
    /// How surveys are turned into heat maps.
    pub survey_settings: MeasuredSettings,
    /// Settings of the active floor.
    pub floor: FloorInfo,
    /// Position of the active floor in the building (0 = lowest).
    pub floor_index: usize,
    /// The other floors, bottom to top, without the active one.
    pub other_floors: Vec<Floor>,
    /// Include APs of nearby floors (through the slabs) in predictions.
    pub cross_floor: bool,
}

fn default_true() -> bool {
    true
}

/// On-disk form of [`Project`]: shared settings plus the ordered list of
/// floors. Format versions 1 and 2 had a single floor at the top level
/// (`plan`, `walls`, `aps`, `surveys`); those are read as one floor.
#[derive(Serialize, Deserialize)]
struct ProjectRepr {
    name: String,
    materials: MaterialLibrary,
    settings: PropagationSettings,
    requirements: Requirements,
    grid_cell_m: f32,
    #[serde(default)]
    survey_settings: MeasuredSettings,
    #[serde(default = "default_true")]
    cross_floor: bool,
    /// Floor shown when the project is opened.
    #[serde(default)]
    active_floor: usize,
    #[serde(default)]
    floors: Vec<Floor>,
    // Format versions 1–2.
    #[serde(default, skip_serializing)]
    plan: Option<FloorPlan>,
    #[serde(default, skip_serializing)]
    walls: Vec<Wall>,
    #[serde(default, skip_serializing)]
    aps: Vec<AccessPoint>,
    #[serde(default, skip_serializing)]
    surveys: Vec<Survey>,
}

impl TryFrom<ProjectRepr> for Project {
    type Error = String;

    fn try_from(r: ProjectRepr) -> Result<Self, String> {
        let mut floors = r.floors;
        if floors.is_empty() {
            let plan = r.plan.ok_or("project has no floors")?;
            floors.push(Floor {
                info: FloorInfo::default(),
                plan,
                walls: r.walls,
                aps: r.aps,
                surveys: r.surveys,
            });
        }
        let active = r.active_floor.min(floors.len() - 1);
        let current = floors.remove(active);
        Ok(Self {
            name: r.name,
            plan: current.plan,
            materials: r.materials,
            walls: current.walls,
            aps: current.aps,
            settings: r.settings,
            requirements: r.requirements,
            grid_cell_m: r.grid_cell_m,
            surveys: current.surveys,
            survey_settings: r.survey_settings,
            floor: current.info,
            floor_index: active,
            other_floors: floors,
            cross_floor: r.cross_floor,
        })
    }
}

impl From<Project> for ProjectRepr {
    fn from(p: Project) -> Self {
        let floors = p.floors();
        Self {
            name: p.name,
            materials: p.materials,
            settings: p.settings,
            requirements: p.requirements,
            grid_cell_m: p.grid_cell_m,
            survey_settings: p.survey_settings,
            cross_floor: p.cross_floor,
            active_floor: p.floor_index,
            floors,
            plan: None,
            walls: Vec::new(),
            aps: Vec::new(),
            surveys: Vec::new(),
        }
    }
}

impl Project {
    pub fn new(name: impl Into<String>, width_px: u32, height_px: u32) -> Self {
        Self {
            name: name.into(),
            plan: FloorPlan {
                width_px,
                height_px,
                calibration: None,
            },
            materials: MaterialLibrary::default(),
            walls: Vec::new(),
            aps: Vec::new(),
            settings: PropagationSettings::default(),
            requirements: Requirements::default(),
            grid_cell_m: 0.25,
            surveys: Vec::new(),
            survey_settings: MeasuredSettings::default(),
            floor: FloorInfo::default(),
            floor_index: 0,
            other_floors: Vec::new(),
            cross_floor: true,
        }
    }

    pub fn floor_count(&self) -> usize {
        self.other_floors.len() + 1
    }

    /// Settings of floor `i` (building order).
    pub fn floor_info(&self, i: usize) -> &FloorInfo {
        match i.cmp(&self.floor_index) {
            std::cmp::Ordering::Equal => &self.floor,
            std::cmp::Ordering::Less => &self.other_floors[i].info,
            std::cmp::Ordering::Greater => &self.other_floors[i - 1].info,
        }
    }

    /// APs of floor `i` (building order), in that floor's coordinates.
    pub fn floor_aps(&self, i: usize) -> &[AccessPoint] {
        match i.cmp(&self.floor_index) {
            std::cmp::Ordering::Equal => &self.aps,
            std::cmp::Ordering::Less => &self.other_floors[i].aps,
            std::cmp::Ordering::Greater => &self.other_floors[i - 1].aps,
        }
    }

    /// Height of floor `i`'s level above the lowest floor's level.
    pub fn floor_elevation_m(&self, i: usize) -> f32 {
        (0..i).map(|k| self.floor_info(k).height_m).sum()
    }

    /// Total slab loss between floors `a` and `b`.
    pub fn slab_loss_between(&self, a: usize, b: usize) -> f32 {
        let (lo, hi) = (a.min(b), a.max(b));
        (lo + 1..=hi).map(|k| self.floor_info(k).slab_loss_db).sum()
    }

    fn take_active(&mut self) -> Floor {
        Floor {
            info: std::mem::take(&mut self.floor),
            plan: self.plan.clone(),
            walls: std::mem::take(&mut self.walls),
            aps: std::mem::take(&mut self.aps),
            surveys: std::mem::take(&mut self.surveys),
        }
    }

    fn put_active(&mut self, f: Floor) {
        self.floor = f.info;
        self.plan = f.plan;
        self.walls = f.walls;
        self.aps = f.aps;
        self.surveys = f.surveys;
    }

    /// All floors, bottom to top (copies).
    pub fn floors(&self) -> Vec<Floor> {
        let mut all = self.other_floors.clone();
        let active = Floor {
            info: self.floor.clone(),
            plan: self.plan.clone(),
            walls: self.walls.clone(),
            aps: self.aps.clone(),
            surveys: self.surveys.clone(),
        };
        all.insert(self.floor_index, active);
        all
    }

    /// Makes floor `i` the active one (clamped to the last floor).
    pub fn set_active_floor(&mut self, i: usize) {
        let i = i.min(self.floor_count() - 1);
        if i == self.floor_index {
            return;
        }
        let current = self.take_active();
        self.other_floors.insert(self.floor_index, current);
        let next = self.other_floors.remove(i);
        self.put_active(next);
        self.floor_index = i;
    }

    /// A copy of the project with floor `i` active.
    pub fn with_active_floor(&self, i: usize) -> Project {
        let mut p = self.clone();
        p.set_active_floor(i);
        p
    }

    /// Inserts a floor at position `i` (building order; `floor_count()`
    /// puts it on top). The active floor stays active.
    pub fn insert_floor(&mut self, i: usize, floor: Floor) {
        let i = i.min(self.floor_count());
        if i <= self.floor_index {
            self.other_floors.insert(i, floor);
            self.floor_index += 1;
        } else {
            self.other_floors.insert(i - 1, floor);
        }
    }

    /// Removes floor `i`; the only floor cannot be removed. If it was the
    /// active one, the floor below it (or above, for the lowest) becomes
    /// active.
    pub fn remove_floor(&mut self, i: usize) -> Option<Floor> {
        if self.floor_count() < 2 || i >= self.floor_count() {
            return None;
        }
        match i.cmp(&self.floor_index) {
            std::cmp::Ordering::Equal => {
                let removed = self.take_active();
                let next = i.saturating_sub(1);
                let f = self.other_floors.remove(next);
                self.put_active(f);
                self.floor_index = next;
                Some(removed)
            }
            std::cmp::Ordering::Less => {
                self.floor_index -= 1;
                Some(self.other_floors.remove(i))
            }
            std::cmp::Ordering::Greater => Some(self.other_floors.remove(i - 1)),
        }
    }

    /// The APs that radiate into the active floor: its own, plus (when
    /// `cross_floor` is on) those of floors up to [`MAX_FLOORS_AWAY`]
    /// above and below, moved into this floor's coordinates. Their height
    /// becomes relative to this floor's level (negative below it), so the
    /// vertical distance enters the path loss, and the slabs in between
    /// are taken off their transmit power. Walls of the active floor apply
    /// along the whole path; walls of the other floor are not modelled.
    pub fn radiating_aps(&self) -> Vec<AccessPoint> {
        let mut out = self.aps.clone();
        if !self.cross_floor {
            return out;
        }
        let f = self.floor_index;
        for g in 0..self.floor_count() {
            if g == f || g.abs_diff(f) > MAX_FLOORS_AWAY {
                continue;
            }
            let info = self.floor_info(g);
            let dz = self.floor_elevation_m(g) - self.floor_elevation_m(f);
            let shift = info.offset_m - self.floor.offset_m;
            let slab = Db(self.slab_loss_between(f, g));
            let label = info.label(g);
            for ap in self.floor_aps(g) {
                let mut ap = ap.clone();
                ap.position = ap.position + shift;
                ap.height_m += dz;
                ap.name = format!("{} · {label}", ap.name);
                for r in &mut ap.radios {
                    r.tx_power = r.tx_power - slab;
                }
                out.push(ap);
            }
        }
        out
    }

    /// Changes the plan calibration. Walls and APs keep their position on
    /// the image, so their coordinates in meters are rescaled.
    pub fn set_calibration(&mut self, calibration: Option<Calibration>) {
        let old = self.plan.px_per_m();
        self.plan.calibration = calibration;
        let k = old / self.plan.px_per_m();
        if (k - 1.0).abs() < f32::EPSILON {
            return;
        }
        for w in &mut self.walls {
            w.segment.a = w.segment.a * k;
            w.segment.b = w.segment.b * k;
        }
        for ap in &mut self.aps {
            ap.position = ap.position * k;
        }
        for m in self.surveys.iter_mut().flat_map(|s| &mut s.measurements) {
            m.position = m.position * k;
        }
    }

    /// A new AP id, unique across all floors.
    pub fn next_ap_id(&self) -> ApId {
        let ids = self
            .aps
            .iter()
            .chain(self.other_floors.iter().flat_map(|f| &f.aps));
        ApId(ids.map(|a| a.id.0).max().map_or(1, |m| m + 1))
    }

    pub fn ap(&self, id: ApId) -> Option<&AccessPoint> {
        self.aps.iter().find(|a| a.id == id)
    }

    pub fn ap_mut(&mut self, id: ApId) -> Option<&mut AccessPoint> {
        self.aps.iter_mut().find(|a| a.id == id)
    }

    /// True if any wall, on any floor, uses the material.
    pub fn material_in_use(&self, id: MaterialId) -> bool {
        self.walls
            .iter()
            .chain(self.other_floors.iter().flat_map(|f| &f.walls))
            .any(|w| w.material == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Segment;

    #[test]
    fn calibration_scale() {
        let c = Calibration::new(Vec2::new(100.0, 100.0), Vec2::new(400.0, 500.0), 10.0).unwrap();
        assert!((c.px_per_m() - 50.0).abs() < 1e-4);
        assert!(Calibration::new(Vec2::new(1.0, 1.0), Vec2::new(1.0, 1.0), 5.0).is_none());
        assert!(Calibration::new(Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), 0.0).is_none());
    }

    #[test]
    fn recalibration_keeps_pixel_positions() {
        let mut p = Project::new("t", 1000, 800);
        p.walls.push(Wall {
            segment: Segment::new(Vec2::new(1.0, 1.0), Vec2::new(5.0, 1.0)),
            material: MaterialId(1),
            thickness_cm: None,
        });
        let px_before = p.plan.m_to_px(p.walls[0].segment.b);
        p.set_calibration(Calibration::new(
            Vec2::new(0.0, 0.0),
            Vec2::new(500.0, 0.0),
            10.0,
        ));
        let px_after = p.plan.m_to_px(p.walls[0].segment.b);
        assert!(px_before.distance(px_after) < 1e-3);
        assert!((p.walls[0].segment.b.x - 2.0).abs() < 1e-4); // 100 px at 50 px/m
    }

    #[test]
    fn plan_size_in_meters() {
        let p = Project::new("t", 2000, 1000);
        assert_eq!(p.plan.size_m(), (100.0, 50.0));
    }

    fn named_floor(name: &str, ap_id: u32) -> Floor {
        let mut f = Floor::new(
            FloorInfo {
                name: name.into(),
                ..Default::default()
            },
            400,
            200,
        );
        f.aps.push(crate::grid::tests::test_ap(ap_id, 5.0, 5.0, 36));
        f
    }

    /// Three floors: "B" (active, AP 1), "1" (AP 2), "2" (AP 3).
    fn three_floors() -> Project {
        let mut p = Project::new("t", 400, 200);
        p.floor.name = "B".into();
        p.aps.push(crate::grid::tests::test_ap(1, 5.0, 5.0, 36));
        p.insert_floor(1, named_floor("1", 2));
        p.insert_floor(2, named_floor("2", 3));
        p
    }

    #[test]
    fn switching_floors_keeps_their_content() {
        let mut p = three_floors();
        assert_eq!(p.floor_count(), 3);
        p.set_active_floor(2);
        assert_eq!((p.floor_index, p.floor.name.as_str()), (2, "2"));
        assert_eq!(p.aps[0].id, ApId(3));
        p.walls.push(Wall {
            segment: Segment::new(Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0)),
            material: MaterialId(1),
            thickness_cm: None,
        });
        p.set_active_floor(0);
        assert_eq!(p.aps[0].id, ApId(1));
        assert!(p.walls.is_empty());
        let names: Vec<String> = p.floors().into_iter().map(|f| f.info.name).collect();
        assert_eq!(names, ["B", "1", "2"]);
        assert_eq!(p.floors()[2].walls.len(), 1);
        assert_eq!(p.next_ap_id(), ApId(4));
    }

    #[test]
    fn inserting_and_removing_floors() {
        let mut p = three_floors();
        p.set_active_floor(1);
        p.insert_floor(0, named_floor("C", 9));
        assert_eq!((p.floor_index, p.floor.name.as_str()), (2, "1"));
        assert_eq!(p.remove_floor(2).unwrap().info.name, "1");
        // The floor below became active.
        assert_eq!((p.floor_index, p.floor.name.as_str()), (1, "B"));
        assert_eq!(p.remove_floor(0).unwrap().info.name, "C");
        assert_eq!(p.floor_index, 0);
        assert_eq!(p.remove_floor(1).unwrap().info.name, "2");
        assert!(p.remove_floor(0).is_none(), "the only floor stays");
    }

    #[test]
    fn nearby_floors_radiate_through_the_slabs() {
        let mut p = three_floors();
        p.set_active_floor(1);
        p.floor.offset_m = Vec2::new(1.0, 0.0);
        let aps = p.radiating_aps();
        assert_eq!(aps.len(), 3);
        let below = aps.iter().find(|a| a.id == ApId(1)).unwrap();
        let above = aps.iter().find(|a| a.id == ApId(3)).unwrap();
        // 3 m floors, APs 3 m above their floor.
        assert!((below.height_m - 0.0).abs() < 1e-4, "{below:?}");
        assert!((above.height_m - 6.0).abs() < 1e-4, "{above:?}");
        // One 15 dB slab each way (this floor's slab and the one above).
        assert!((below.radios[0].tx_power.0 - 2.0).abs() < 1e-4);
        assert!((above.radios[0].tx_power.0 - 2.0).abs() < 1e-4);
        // Moved into this floor's coordinates.
        assert!((below.position.x - 4.0).abs() < 1e-4);
        assert!(below.name.ends_with("· B"), "{}", below.name);

        p.set_active_floor(0);
        let two_up = p
            .radiating_aps()
            .into_iter()
            .find(|a| a.id == ApId(3))
            .unwrap();
        assert!((two_up.radios[0].tx_power.0 - (17.0 - 30.0)).abs() < 1e-4);
        assert!((two_up.height_m - 9.0).abs() < 1e-4);

        p.cross_floor = false;
        assert_eq!(p.radiating_aps().len(), 1);
    }

    #[test]
    fn serializes_floors_in_building_order() {
        let mut p = three_floors();
        p.set_active_floor(1);
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["active_floor"], 1);
        assert_eq!(json["floors"].as_array().unwrap().len(), 3);
        assert_eq!(json["floors"][0]["info"]["name"], "B");
        assert!(json.get("walls").is_none());
        let back: Project = serde_json::from_value(json).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn single_floor_projects_from_format_2_load() {
        let mut v2 = serde_json::to_value(Project::new("old", 100, 50)).unwrap();
        let floor = v2["floors"][0].clone();
        let obj = v2.as_object_mut().unwrap();
        obj.remove("floors");
        obj.remove("active_floor");
        obj.remove("cross_floor");
        obj.insert("plan".into(), floor["plan"].clone());
        obj.insert("walls".into(), serde_json::json!([]));
        obj.insert("aps".into(), serde_json::json!([]));
        let p: Project = serde_json::from_value(v2).unwrap();
        assert_eq!(p.floor_count(), 1);
        assert_eq!(p.plan.width_px, 100);
        assert_eq!(p.floor, FloorInfo::default());
        assert!(p.cross_floor);
    }
}
