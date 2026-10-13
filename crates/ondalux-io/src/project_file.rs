//! `.lux` project files. Format specification: `docs/project-format.md`.
//!
//! A `.lux` file is a ZIP archive containing:
//! - `project.json`: header (`format`, `format_version`) + the project model;
//! - `floorplan.<ext>`: the original floor plan image bytes (optional).

use crate::error::IoError;
use ondalux_core::project::Project;
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, Write};
use std::path::Path;

pub const FORMAT_ID: &str = "ondalux-project";
/// Version history: 1 = Phase 1 model; 2 = adds `surveys` (v1 files load
/// unchanged, with no surveys).
pub const FORMAT_VERSION: u32 = 2;
pub const EXTENSION: &str = "lux";
/// Format id and extension used before the project was renamed. Still read.
pub const LEGACY_FORMAT_ID: &str = "heatcraft-project";
pub const LEGACY_EXTENSION: &str = "hcraft";

const PROJECT_ENTRY: &str = "project.json";
const PLAN_PREFIX: &str = "floorplan.";

/// Floor plan image as stored in the project (original encoded bytes).
#[derive(Debug, Clone, PartialEq)]
pub struct PlanImage {
    /// Lowercase file extension, e.g. `png` or `jpg`.
    pub extension: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectFile {
    pub project: Project,
    pub plan_image: Option<PlanImage>,
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
        if let Some(img) = &self.plan_image {
            // Images are already compressed; store them as-is.
            let stored = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip.start_file(format!("{PLAN_PREFIX}{}", img.extension), stored)?;
            zip.write_all(&img.bytes)?;
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
        // Older versions would be migrated here, one step at a time.
        let project: Project = serde_json::from_value(header.project)?;

        let plan_name = zip
            .file_names()
            .find(|n| n.starts_with(PLAN_PREFIX))
            .map(str::to_owned);
        let plan_image = match plan_name {
            Some(name) => {
                let mut bytes = Vec::new();
                zip.by_name(&name)?.read_to_end(&mut bytes)?;
                Some(PlanImage {
                    extension: name[PLAN_PREFIX.len()..].to_ascii_lowercase(),
                    bytes,
                })
            }
            None => None,
        };
        Ok(Self {
            project,
            plan_image,
        })
    }
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
        });
        p.aps.push(AccessPoint {
            id: ApId(1),
            name: "Lobby".into(),
            model: "Generic Wi-Fi 6".into(),
            position: Vec2::new(4.0, 5.0),
            height_m: 2.8,
            orientation_deg: 90.0,
            radios: vec![Radio {
                channel: Channel::new(Band::G6, 37, ChannelWidth::Mhz80).unwrap(),
                tx_power: Dbm(18.0),
                antenna_gain: Db(4.5),
                enabled: true,
            }],
        });
        ProjectFile {
            project: p,
            plan_image: Some(PlanImage {
                extension: "png".into(),
                bytes: vec![0x89, b'P', b'N', b'G', 1, 2, 3],
            }),
        }
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
            "project": serde_json::to_value(&sample().project).unwrap(),
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
    fn rejects_foreign_files() {
        let json = r#"{"format":"something-else","format_version":1,"project":{}}"#;
        assert!(matches!(
            ProjectFile::read_from(archive_with(json)),
            Err(IoError::Format(_))
        ));
        assert!(ProjectFile::read_from(Cursor::new(b"not a zip".to_vec())).is_err());
    }
}
