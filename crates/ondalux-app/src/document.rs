//! An open project plus its editing state (file path, dirty flag, undo).
//!
//! Undo uses whole-project snapshots: the project model excludes the floor
//! plan images, which snapshots share (`Arc`), so a snapshot is small
//! (walls, APs, settings).

use ondalux_core::project::{Floor, Project};
use ondalux_io::{FloorAssets, PlanImage, ProjectFile};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

const UNDO_LIMIT: usize = 200;
/// Edits with the same key within this window share one undo step
/// (e.g. dragging a slider).
const COALESCE: Duration = Duration::from_millis(1500);

#[derive(Clone)]
struct Snapshot {
    project: Project,
    floors: Vec<Arc<FloorAssets>>,
}

pub struct Document {
    pub project: Project,
    /// Plan files of each floor, in building order.
    pub floors: Vec<Arc<FloorAssets>>,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    last_key: Option<(&'static str, Instant)>,
}

impl Document {
    pub fn new(project: Project, plan_image: Option<PlanImage>, path: Option<PathBuf>) -> Self {
        let floors = vec![Arc::new(FloorAssets {
            plan_image,
            plan_source: None,
        })];
        let mut doc = Self {
            project,
            floors,
            path,
            dirty: false,
            undo: Vec::new(),
            redo: Vec::new(),
            last_key: None,
        };
        doc.pad_floors();
        doc
    }

    pub fn from_file(file: ProjectFile, path: PathBuf) -> Self {
        let mut doc = Self::new(file.project, None, Some(path));
        doc.floors = file.floors.into_iter().map(Arc::new).collect();
        doc.pad_floors();
        doc
    }

    /// One assets entry per floor.
    fn pad_floors(&mut self) {
        let n = self.project.floor_count();
        self.floors.resize_with(n, Default::default);
    }

    pub fn to_file(&self) -> ProjectFile {
        ProjectFile {
            project: self.project.clone(),
            floors: self.floors.iter().map(|a| (**a).clone()).collect(),
        }
    }

    /// Plan files of the active floor.
    pub fn assets(&self) -> &Arc<FloorAssets> {
        &self.floors[self.project.floor_index]
    }

    pub fn plan_image(&self) -> Option<&PlanImage> {
        self.assets().plan_image.as_ref()
    }

    /// Original PDF of the active floor's plan, if it came from one.
    pub fn plan_source(&self) -> Option<&Vec<u8>> {
        self.assets().plan_source.as_ref()
    }

    /// Replaces the active floor's plan files (no undo step).
    pub fn set_assets(&mut self, assets: FloorAssets) {
        let i = self.project.floor_index;
        self.floors[i] = Arc::new(assets);
    }

    /// Adds a floor at `index` (building order) and makes it active.
    pub fn add_floor(&mut self, index: usize, floor: Floor, assets: FloorAssets) {
        self.checkpoint();
        let index = index.min(self.project.floor_count());
        self.project.insert_floor(index, floor);
        self.floors.insert(index, Arc::new(assets));
        self.project.set_active_floor(index);
    }

    /// Removes floor `index` (never the only one).
    pub fn remove_floor(&mut self, index: usize) -> bool {
        if self.project.floor_count() < 2 || index >= self.project.floor_count() {
            return false;
        }
        self.checkpoint();
        self.project.remove_floor(index);
        self.floors.remove(index);
        true
    }

    pub fn title(&self) -> String {
        let name = self.path.as_ref().and_then(|p| p.file_name()).map_or_else(
            || self.project.name.clone(),
            |n| n.to_string_lossy().into_owned(),
        );
        if self.dirty {
            format!("{name} •")
        } else {
            name
        }
    }

    /// Saves an undo snapshot. Call before every discrete user edit.
    pub fn checkpoint(&mut self) {
        self.last_key = None;
        self.push_undo();
    }

    /// Like [`Self::checkpoint`], but consecutive edits with the same `key`
    /// in a short time window share a single undo step.
    pub fn checkpoint_coalesced(&mut self, key: &'static str) {
        let now = Instant::now();
        let merge = self
            .last_key
            .is_some_and(|(k, t)| k == key && now.duration_since(t) < COALESCE);
        if !merge {
            self.push_undo();
        }
        self.last_key = Some((key, now));
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            project: self.project.clone(),
            floors: self.floors.clone(),
        }
    }

    fn restore(&mut self, s: Snapshot) -> Snapshot {
        let current = self.snapshot();
        self.project = s.project;
        self.floors = s.floors;
        current
    }

    fn push_undo(&mut self) {
        self.undo.push(self.snapshot());
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.dirty = true;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.undo.pop() else {
            return false;
        };
        let current = self.restore(prev);
        self.redo.push(current);
        self.last_key = None;
        self.dirty = true;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        let current = self.restore(next);
        self.undo.push(current);
        self.last_key = None;
        self.dirty = true;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_redo_and_coalescing() {
        let mut d = Document::new(Project::new("a", 10, 10), None, None);
        d.checkpoint();
        d.project.name = "b".into();
        d.checkpoint_coalesced("name");
        d.project.name = "c".into();
        d.checkpoint_coalesced("name");
        d.project.name = "d".into();
        assert!(d.undo());
        assert_eq!(d.project.name, "b");
        assert!(d.undo());
        assert_eq!(d.project.name, "a");
        assert!(!d.undo());
        assert!(d.redo());
        assert_eq!(d.project.name, "b");
    }

    #[test]
    fn floors_and_their_plans_undo_together() {
        let img = |b: u8| PlanImage {
            extension: "png".into(),
            bytes: vec![b],
        };
        let mut d = Document::new(Project::new("a", 10, 10), Some(img(1)), None);
        let floor = Floor::new(Default::default(), 20, 20);
        d.add_floor(
            1,
            floor,
            FloorAssets {
                plan_image: Some(img(2)),
                plan_source: None,
            },
        );
        assert_eq!(d.project.floor_index, 1);
        assert_eq!(d.plan_image().unwrap().bytes, [2]);
        assert!(d.undo());
        assert_eq!(d.project.floor_count(), 1);
        assert_eq!(d.floors.len(), 1);
        assert_eq!(d.plan_image().unwrap().bytes, [1]);
        assert!(d.redo());
        assert!(d.remove_floor(0));
        assert_eq!(d.plan_image().unwrap().bytes, [2]);
        assert!(!d.remove_floor(0), "the only floor stays");
    }
}
