//! `.lux` project files. Format specification: `docs/project-format.md`.
//!
//! A `.lux` file is a ZIP archive containing:
//! - `project.json`: header (`format`, `format_version`) + the project model;
//! - `floorplan.<ext>` / `floorplan-source.pdf`: the plan image and the
//!   original PDF of the lowest floor (optional);
//! - `floors/<n>/floorplan.<ext>` / `floors/<n>/floorplan-source.pdf`: the
//!   same for floor `n` (0-based, building order), `n` ≥ 1;
//! - `[floors/<n>/]floorplan-source.json` (optional): `{"page": p,
//!   "same_as": m}` — the plan is page `p` of the PDF, and the PDF is the
//!   one stored for floor `m` (several floors from one multi-page PDF
//!   store it once).

use crate::error::IoError;
use ondalux_core::project::Project;
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, Write};
use std::path::Path;

pub const FORMAT_ID: &str = "ondalux-project";
/// Version history: 1 = Phase 1 model; 2 = adds `surveys` (v1 files load
/// unchanged, with no surveys); 3 = multi-floor buildings (`floors`; v1–2
/// files load as a single floor).
pub const FORMAT_VERSION: u32 = 3;
pub const EXTENSION: &str = "lux";
/// Format id and extension used before the project was renamed. Still read.
pub const LEGACY_FORMAT_ID: &str = "heatcraft-project";
pub const LEGACY_EXTENSION: &str = "hcraft";

const PROJECT_ENTRY: &str = "project.json";
const PLAN_PREFIX: &str = "floorplan.";
/// Original PDF of the plan (optional; its line art feeds wall detection).
const PLAN_SOURCE_ENTRY: &str = "floorplan-source.pdf";

/// Floor plan image as stored in the project (original encoded bytes).
#[derive(Debug, Clone, PartialEq)]
pub struct PlanImage {
    /// Lowercase file extension, e.g. `png` or `jpg`.
    pub extension: String,
    pub bytes: Vec<u8>,
}

/// The files of one floor.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FloorAssets {
    pub plan_image: Option<PlanImage>,
    /// Original PDF of the plan, kept for wall detection (optional).
    pub plan_source: Option<Vec<u8>>,
    /// Page of `plan_source` with this floor's plan (0-based).
    pub plan_source_page: usize,
}

/// Sidecar of a floor's source PDF.
#[derive(Serialize, Deserialize, Default)]
struct SourceInfo {
    #[serde(default)]
    page: usize,
    /// Floor whose entry holds the PDF bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    same_as: Option<usize>,
}

const SOURCE_INFO_ENTRY: &str = "floorplan-source.json";

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectFile {
    pub project: Project,
    /// One entry per floor, in building order (missing entries = no files).
    pub floors: Vec<FloorAssets>,
}

impl ProjectFile {
    /// A single-floor project with its plan image.
    pub fn single(project: Project, plan_image: Option<PlanImage>) -> Self {
        Self {
            project,
            floors: vec![FloorAssets {
                plan_image,
                ..Default::default()
            }],
        }
    }

    /// Plan image of floor `i` (building order).
    pub fn plan_image(&self, i: usize) -> Option<&PlanImage> {
        self.floors.get(i)?.plan_image.as_ref()
    }
}

/// Folder of floor `i`'s files inside the archive.
fn floor_dir(i: usize) -> String {
    if i == 0 {
        String::new()
    } else {
        format!("floors/{i}/")
    }
}

#[derive(Serialize)]
struct HeaderOut<'a> {
    format: &'a str,
    format_version: u32,
    generator: String,
    project: &'a Project,
}

#[derive(Deserialize)]
struct HeaderIn {
    format: String,
    format_version: u32,
    project: serde_json::Value,
}

impl ProjectFile {
    pub fn save(&self, path: &Path) -> Result<(), IoError> {
        // Write to a temporary file first so a failed save never corrupts
        // the previous version.
        let tmp = path.with_extension(format!("{EXTENSION}.tmp"));
        {
            let file = std::fs::File::create(&tmp)?;
            self.write_to(std::io::BufWriter::new(file))?;
        }
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, IoError> {
        let file = std::fs::File::open(path)?;
        Self::read_from(std::io::BufReader::new(file))
    }

    pub fn write_to<W: Write + Seek>(&self, writer: W) -> Result<(), IoError> {
        let mut zip = zip::ZipWriter::new(writer);
        let deflate = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zip.start_file(PROJECT_ENTRY, deflate)?;
        let header = HeaderOut {
            format: FORMAT_ID,
            format_version: FORMAT_VERSION,
            generator: format!("Ondalux Heatmap {}", env!("CARGO_PKG_VERSION")),
            project: &self.project,
        };
        serde_json::to_writer_pretty(&mut zip, &header)?;
        for (i, assets) in self.floors.iter().enumerate() {
            let dir = floor_dir(i);
            if let Some(img) = &assets.plan_image {
                // Images are already compressed; store them as-is.
                let stored = zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored);
                zip.start_file(format!("{dir}{PLAN_PREFIX}{}", img.extension), stored)?;
                zip.write_all(&img.bytes)?;
            }
            if let Some(pdf) = &assets.plan_source {
                let same_as = self.floors[..i]
                    .iter()
                    .position(|f| f.plan_source.as_ref() == Some(pdf));
                if same_as.is_none() {
                    zip.start_file(format!("{dir}{PLAN_SOURCE_ENTRY}"), deflate)?;
                    zip.write_all(pdf)?;
                }
                if same_as.is_some() || assets.plan_source_page != 0 {
                    let info = SourceInfo {
                        page: assets.plan_source_page,
                        same_as,
                    };
                    zip.start_file(format!("{dir}{SOURCE_INFO_ENTRY}"), deflate)?;
                    serde_json::to_writer(&mut zip, &info)?;
                }
            }
        }
        zip.finish()?;
        Ok(())
    }

    pub fn read_from<R: Read + Seek>(reader: R) -> Result<Self, IoError> {
        let mut zip = zip::ZipArchive::new(reader)?;
        let header: HeaderIn = {
            let entry = zip
                .by_name(PROJECT_ENTRY)
                .map_err(|_| IoError::Format(format!("missing {PROJECT_ENTRY}")))?;
            serde_json::from_reader(entry)?
        };
        if header.format != FORMAT_ID && header.format != LEGACY_FORMAT_ID {
            return Err(IoError::Format(format!(
                "not an Ondalux Heatmap project (format = {:?})",
                header.format
            )));
        }
        if header.format_version > FORMAT_VERSION {
            return Err(IoError::NewerVersion {
                found: header.format_version,
                supported: FORMAT_VERSION,
            });
        }
        // Versions 1–2 (single floor at the top level) are read by the
        // project model itself.
        let mut project: Project = serde_json::from_value(header.project)?;
        if header.format_version < 3 {
            project.materials.fill_reference_thickness();
        }
        let mut floors = Vec::with_capacity(project.floor_count());
        for i in 0..project.floor_count() {
            let mut assets = read_floor_assets(&mut zip, &floor_dir(i))?;
            let info: SourceInfo =
                match zip.by_name(&format!("{}{SOURCE_INFO_ENTRY}", floor_dir(i))) {
                    Ok(entry) => serde_json::from_reader(entry)?,
                    Err(_) => SourceInfo::default(),
                };
            assets.plan_source_page = info.page;
            if let Some(m) = info.same_as.filter(|&m| m < i) {
                assets.plan_source = floors
                    .get(m)
                    .and_then(|f: &FloorAssets| f.plan_source.clone());
            }
            floors.push(assets);
        }
        Ok(Self { project, floors })
    }
}

fn read_floor_assets<R: Read + Seek>(
    zip: &mut zip::ZipArchive<R>,
    dir: &str,
) -> Result<FloorAssets, IoError> {
    let prefix = format!("{dir}{PLAN_PREFIX}");
    let plan_name = zip
        .file_names()
        .find(|n| n.starts_with(&prefix) && !n[prefix.len()..].contains('/'))
        .map(str::to_owned);
    let plan_image = match plan_name {
        Some(name) => {
            let mut bytes = Vec::new();
            zip.by_name(&name)?.read_to_end(&mut bytes)?;
            Some(PlanImage {
                extension: name[prefix.len()..].to_ascii_lowercase(),
                bytes,
            })
        }
        None => None,
    };
    let plan_source = match zip.by_name(&format!("{dir}{PLAN_SOURCE_ENTRY}")) {
        Ok(mut entry) => {
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            Some(bytes)
        }
        Err(_) => None,
    };
    Ok(FloorAssets {
        plan_image,
        plan_source,
        plan_source_page: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ondalux_core::ap::{AccessPoint, ApId, Radio};
    use ondalux_core::channel::{Band, Channel, ChannelWidth};
    use ondalux_core::geometry::{Segment, Vec2};
    use ondalux_core::material::{MaterialId, Wall};
    use ondalux_core::project::Calibration;
    use ondalux_core::units::{Db, Dbm};
    use std::io::Cursor;

    fn sample() -> ProjectFile {
        let mut p = Project::new("Office", 1200, 800);
        p.set_calibration(Calibration::new(
            Vec2::new(0.0, 0.0),
            Vec2::new(300.0, 0.0),
            10.0,
        ));
        p.walls.push(Wall {
            segment: Segment::new(Vec2::new(1.0, 2.0), Vec2::new(8.0, 2.0)),
            material: MaterialId(5),
            thickness_cm: None,
        });
        p.aps.push(AccessPoint {
            id: ApId(1),
            name: "Lobby".into(),
            vendor: String::new(),
            model: "Generic Wi-Fi 6".into(),
            position: Vec2::new(4.0, 5.0),
            height_m: 2.8,
            orientation_deg: 90.0,
            mount: Default::default(),
            pattern: Default::default(),
            radios: vec![Radio {
                channel: Channel::new(Band::G6, 37, ChannelWidth::Mhz80).unwrap(),
                tx_power: Dbm(18.0),
                antenna_gain: Db(4.5),
                enabled: true,
            }],
        });
        ProjectFile {
            project: p,
            floors: vec![FloorAssets {
                plan_image: Some(PlanImage {
                    extension: "png".into(),
                    bytes: vec![0x89, b'P', b'N', b'G', 1, 2, 3],
                }),
                plan_source: Some(b"%PDF-1.4 test".to_vec()),
                plan_source_page: 0,
            }],
        }
    }

    /// The project as format versions 1–2 stored it: one floor at the top
    /// level.
    fn legacy_project_json(p: &Project) -> serde_json::Value {
        let mut v = serde_json::to_value(p).unwrap();
        let floor = v["floors"][0].clone();
        let obj = v.as_object_mut().unwrap();
        for k in ["floors", "active_floor", "cross_floor"] {
            obj.remove(k);
        }
        for k in ["plan", "walls", "aps", "surveys"] {
            obj.insert(k.into(), floor[k].clone());
        }
        v
    }

    #[test]
    fn round_trip_with_several_floors() {
        let mut original = sample();
        let mut upper = ondalux_core::project::Floor::new(Default::default(), 600, 400);
        upper.info.name = "Mezzanine".into();
        original.project.insert_floor(1, upper.clone());
        original.project.insert_floor(2, upper);
        original.project.set_active_floor(2);
        // Floor 1 has no files, floor 2 an image and page 3 of floor 0's PDF.
        original.floors.push(FloorAssets::default());
        original.floors.push(FloorAssets {
            plan_image: Some(PlanImage {
                extension: "jpg".into(),
                bytes: vec![0xff, 0xd8, 9],
            }),
            plan_source: original.floors[0].plan_source.clone(),
            plan_source_page: 2,
        });
        let mut buf = Cursor::new(Vec::new());
        original.write_to(&mut buf).unwrap();
        buf.set_position(0);
        // The shared PDF is stored once.
        let zip = zip::ZipArchive::new(&mut buf).unwrap();
        let pdfs = zip
            .file_names()
            .filter(|n| n.ends_with(PLAN_SOURCE_ENTRY))
            .count();
        assert_eq!(pdfs, 1);
        drop(zip);
        buf.set_position(0);
        let loaded = ProjectFile::read_from(buf).unwrap();
        assert_eq!(loaded, original);
        assert_eq!(loaded.project.floor_index, 2);
    }

    #[test]
    fn round_trip() {
        let original = sample();
        let mut buf = Cursor::new(Vec::new());
        original.write_to(&mut buf).unwrap();
        buf.set_position(0);
        let loaded = ProjectFile::read_from(buf).unwrap();
        assert_eq!(loaded, original);
    }

    #[test]
    fn round_trip_on_disk() {
        let dir = std::env::temp_dir().join(format!("ondalux-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("p.lux");
        let original = sample();
        original.save(&path).unwrap();
        assert_eq!(ProjectFile::load(&path).unwrap(), original);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn archive_with(json: &str) -> Cursor<Vec<u8>> {
        let mut buf = Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(&mut buf);
        zip.start_file(PROJECT_ENTRY, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(json.as_bytes()).unwrap();
        zip.finish().unwrap();
        buf.set_position(0);
        buf
    }

    #[test]
    fn rejects_newer_versions() {
        let json = r#"{"format":"ondalux-project","format_version":999,"project":{}}"#;
        let err = ProjectFile::read_from(archive_with(json)).unwrap_err();
        assert!(
            matches!(err, IoError::NewerVersion { found: 999, .. }),
            "{err}"
        );
    }

    #[test]
    fn reads_legacy_format_id() {
        let mut buf = Cursor::new(Vec::new());
        sample().write_to(&mut buf).unwrap();
        buf.set_position(0);
        let mut zip = zip::ZipArchive::new(buf).unwrap();
        let mut json = String::new();
        zip.by_name(PROJECT_ENTRY)
            .unwrap()
            .read_to_string(&mut json)
            .unwrap();
        let legacy = json.replace(FORMAT_ID, LEGACY_FORMAT_ID);
        let loaded = ProjectFile::read_from(archive_with(&legacy)).unwrap();
        assert_eq!(loaded.project, sample().project);
    }

    #[test]
    fn loads_version_1_without_surveys() {
        let mut header = serde_json::json!({
            "format": FORMAT_ID,
            "format_version": 1,
            "project": legacy_project_json(&sample().project),
        });
        header["project"]
            .as_object_mut()
            .unwrap()
            .remove("surveys")
            .unwrap();
        let loaded = ProjectFile::read_from(archive_with(&header.to_string())).unwrap();
        assert!(loaded.project.surveys.is_empty());
        assert_eq!(loaded.project.aps, sample().project.aps);
    }

    #[test]
    fn loads_version_2_as_one_floor() {
        let header = serde_json::json!({
            "format": FORMAT_ID,
            "format_version": 2,
            "project": legacy_project_json(&sample().project),
        });
        let loaded = ProjectFile::read_from(archive_with(&header.to_string())).unwrap();
        assert_eq!(loaded.project, sample().project);
        assert_eq!(loaded.project.floor_count(), 1);
        assert_eq!(loaded.floors, vec![FloorAssets::default()]);
    }

    #[test]
    fn rejects_foreign_files() {
        let json = r#"{"format":"something-else","format_version":1,"project":{}}"#;
        assert!(matches!(
            ProjectFile::read_from(archive_with(json)),
            Err(IoError::Format(_))
        ));
        assert!(ProjectFile::read_from(Cursor::new(b"not a zip".to_vec())).is_err());
    }
}
