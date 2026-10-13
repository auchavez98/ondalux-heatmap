//! The user's own AP models, stored next to the application settings:
//! `<config dir>/ondalux/ap-models.json` (same format as the built-in
//! catalog, see `docs/ap-catalog.md`).

use crate::error::IoError;
use ondalux_core::catalog::{ApModel, CatalogFile};
use std::path::{Path, PathBuf};

/// Default location of the user catalog, if the OS has a config directory.
pub fn default_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("ondalux").join("ap-models.json"))
}

/// Reads a catalog file. A missing file is an empty catalog.
pub fn load(path: &Path) -> Result<Vec<ApModel>, IoError> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

/// Parses `{"models": [...]}` or a bare `[...]` list, validating each model.
pub fn parse(text: &str) -> Result<Vec<ApModel>, IoError> {
    let models = match serde_json::from_str::<CatalogFile>(text) {
        Ok(f) => f.models,
        Err(_) => serde_json::from_str::<Vec<ApModel>>(text)?,
    };
    for m in &models {
        m.validate().map_err(IoError::Format)?;
    }
    Ok(models)
}

/// Writes models atomically (temporary file, then rename).
pub fn save(path: &Path, models: &[ApModel]) -> Result<(), IoError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = CatalogFile {
        models: models.to_vec(),
    };
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&file)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ondalux_core::catalog::Catalog;

    #[test]
    fn save_load_round_trip_and_bare_lists() {
        let dir = std::env::temp_dir().join(format!("ondalux-cat-{}", std::process::id()));
        let path = dir.join("nested").join("ap-models.json");
        let models: Vec<ApModel> = Catalog::builtin()
            .search("u6")
            .into_iter()
            .map(|(m, _)| m.clone())
            .collect();
        assert!(!models.is_empty());
        save(&path, &models).unwrap();
        assert_eq!(load(&path).unwrap(), models);
        let bare = serde_json::to_string(&models).unwrap();
        assert_eq!(parse(&bare).unwrap(), models);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_file_is_empty_and_bad_models_are_rejected() {
        assert!(load(Path::new("definitely/not/here.json"))
            .unwrap()
            .is_empty());
        let bad = r#"{"models":[{"vendor":"","model":"X","standard":"Wi-Fi 6","radios":[],"source":"https://x"}]}"#;
        assert!(parse(bad).is_err());
    }
}
