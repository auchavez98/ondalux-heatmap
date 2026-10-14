//! UI language (Spanish by default, English optional) and the small
//! preferences file `<config dir>/ondalux/ui.json`.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};

static ENGLISH: AtomicBool = AtomicBool::new(false);

/// Picks the text for the current UI language.
pub fn t(es: &'static str, en: &'static str) -> &'static str {
    if ENGLISH.load(Ordering::Relaxed) {
        en
    } else {
        es
    }
}

/// `format!` in the current UI language: `tr!("es {x}", "en {x}", ...)`.
#[macro_export]
macro_rules! tr {
    ($es:literal, $en:literal $(, $arg:expr)* $(,)?) => {
        if $crate::i18n::is_english() {
            format!($en $(, $arg)*)
        } else {
            format!($es $(, $arg)*)
        }
    };
}

pub fn is_english() -> bool {
    ENGLISH.load(Ordering::Relaxed)
}

pub fn set_english(on: bool) {
    ENGLISH.store(on, Ordering::Relaxed);
}

/// English names of the built-in materials and their Spanish display names.
const MATERIALS: [(&str, &str); 12] = [
    ("Drywall", "Tablaroca / yeso"),
    ("Wood", "Madera"),
    ("Glass", "Vidrio"),
    ("Brick", "Ladrillo"),
    ("Concrete", "Concreto"),
    ("Metal", "Metal"),
    ("Concrete block", "Block de concreto"),
    ("Low-E glass", "Vidrio Low-E (térmico)"),
    ("Wood door", "Puerta de madera"),
    ("Metal door", "Puerta metálica"),
    ("Elevator shaft", "Cubo de elevador"),
    ("Cubicle partition", "Mampara de oficina"),
];

/// Display name of a material: built-in names follow the UI language,
/// names typed by the user are shown as they are.
pub fn material_name(name: &str) -> &str {
    if is_english() {
        return name;
    }
    MATERIALS
        .iter()
        .find(|(en, _)| *en == name)
        .map_or(name, |(_, es)| es)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct UiPrefs {
    pub english: bool,
    pub theme: ThemeChoice,
}

fn prefs_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|d| d.join("ondalux").join("ui.json"))
}

impl UiPrefs {
    pub fn load() -> Self {
        prefs_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Best effort: preferences are a convenience, failures are ignored.
    pub fn save(&self) {
        if let Some(p) = prefs_path() {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(s) = serde_json::to_string_pretty(self) {
                let _ = std::fs::write(p, s);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switches_language() {
        set_english(false);
        assert_eq!(t("Abrir", "Open"), "Abrir");
        assert_eq!(material_name("Concrete"), "Concreto");
        assert_eq!(material_name("Muro especial"), "Muro especial");
        set_english(true);
        assert_eq!(t("Abrir", "Open"), "Open");
        assert_eq!(material_name("Concrete"), "Concrete");
        set_english(false);
    }

    #[test]
    fn prefs_round_trip_and_defaults() {
        let p = UiPrefs {
            english: true,
            theme: ThemeChoice::Light,
        };
        let s = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<UiPrefs>(&s).unwrap(), p);
        assert_eq!(
            serde_json::from_str::<UiPrefs>("{}").unwrap(),
            UiPrefs::default()
        );
    }
}
