//! An open project plus its editing state (file path, dirty flag, undo).
//!
//! Undo uses whole-project snapshots: the project model excludes the floor
//! plan image, so a snapshot is small (walls, APs, settings).

use ondalux_core::project::Project;
use ondalux_io::{PlanImage, ProjectFile};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const UNDO_LIMIT: usize = 200;
/// Edits with the same key within this window share one undo step
/// (e.g. dragging a slider).
const COALESCE: Duration = Duration::from_millis(1500);

pub struct Document {
    pub project: Project,
    pub plan_image: Option<PlanImage>,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    undo: Vec<Project>,
    redo: Vec<Project>,
    last_key: Option<(&'static str, Instant)>,
}

impl Document {
    pub fn new(project: Project, plan_image: Option<PlanImage>, path: Option<PathBuf>) -> Self {
        Self {
            project,
            plan_image,
            path,
            dirty: false,
            undo: Vec::new(),
            redo: Vec::new(),
            last_key: None,
        }
    }

    pub fn from_file(file: ProjectFile, path: PathBuf) -> Self {
        Self::new(file.project, file.plan_image, Some(path))
    }

    pub fn to_file(&self) -> ProjectFile {
        ProjectFile {
            project: self.project.clone(),
            plan_image: self.plan_image.clone(),
        }
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

    fn push_undo(&mut self) {
        self.undo.push(self.project.clone());
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
        self.redo.push(std::mem::replace(&mut self.project, prev));
        self.last_key = None;
        self.dirty = true;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(std::mem::replace(&mut self.project, next));
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
}
