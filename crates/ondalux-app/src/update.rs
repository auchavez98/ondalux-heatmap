//! Looks for a newer version on the website. The app downloads nothing by
//! itself: it reads `version.json` (version, download page and what is
//! new) and, if there is a newer version, the user decides whether to open
//! the download page, be reminded later or skip that version.
//!
//! The request carries no data beyond what any web request has (IP address
//! and a User-Agent with the app version). Automatic checks happen at most
//! once a day and can be turned off.

use crate::app::OndaluxApp;
use crate::i18n::{is_english, t};
use crate::style::{self, icon};
use crate::tr;
use eframe::egui;
use serde::Deserialize;
use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const VERSION_URL: &str = "https://ondaluxheatmap.com/version.json";
const CHECK_EVERY: Duration = Duration::from_secs(24 * 3600);
const TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default)]
pub struct PerLang<T> {
    pub en: T,
    pub es: T,
}

impl<T> PerLang<T> {
    fn get(&self) -> &T {
        if is_english() {
            &self.en
        } else {
            &self.es
        }
    }
}

/// Contents of `version.json`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Release {
    pub version: String,
    #[serde(default)]
    pub date: String,
    /// Download page per language.
    #[serde(default)]
    pub download: PerLang<String>,
    /// What is new, one line per item.
    #[serde(default)]
    pub notes: PerLang<Vec<String>>,
}

/// "1.2.3" (optionally with a leading "v" or a "-suffix") → (1, 2, 3).
pub fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let s = s.trim().trim_start_matches('v');
    let core = s.split(['-', '+']).next()?;
    let mut it = core.split('.').map(|p| p.parse::<u32>().ok());
    let v = (
        it.next()??,
        it.next().flatten().unwrap_or(0),
        it.next().flatten().unwrap_or(0),
    );
    Some(v)
}

/// True if `remote` is a later version than `current`.
pub fn is_newer(remote: &str, current: &str) -> bool {
    match (parse_version(remote), parse_version(current)) {
        (Some(r), Some(c)) => r > c,
        _ => false,
    }
}

fn now_s() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn fetch() -> Result<Release, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(TIMEOUT)
        .user_agent(&format!("Ondalux-Heatmap/{}", env!("CARGO_PKG_VERSION")))
        .build();
    let body = agent
        .get(VERSION_URL)
        .call()
        .map_err(|e| e.to_string())?
        .into_string()
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&body).map_err(|e| e.to_string())
}

#[derive(Default)]
pub struct UpdateState {
    job: Option<mpsc::Receiver<Result<Release, String>>>,
    /// Started from the menu: report "up to date" and errors too.
    manual: bool,
    pub available: Option<Release>,
    pub open: bool,
}

impl UpdateState {
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }
}

/// Starts a check in the background.
pub fn check(app: &mut OndaluxApp, ctx: &egui::Context, manual: bool) {
    if app.update.busy() {
        return;
    }
    let (tx, rx) = mpsc::channel();
    let ctx = ctx.clone();
    let spawned = std::thread::Builder::new()
        .name("update-check".into())
        .spawn(move || {
            let _ = tx.send(fetch());
            ctx.request_repaint();
        });
    if spawned.is_ok() {
        app.update.job = Some(rx);
        app.update.manual = manual;
    }
}

/// Automatic check at startup, if enabled and not done in the last day.
pub fn check_on_startup(app: &mut OndaluxApp, ctx: &egui::Context) {
    if cfg!(test) || !app.prefs.check_updates {
        return;
    }
    let now = now_s();
    if now.saturating_sub(app.prefs.last_update_check) < CHECK_EVERY.as_secs() {
        return;
    }
    app.prefs.last_update_check = now;
    app.prefs.save();
    check(app, ctx, false);
}

pub fn poll(app: &mut OndaluxApp) {
    let Some(rx) = &app.update.job else {
        return;
    };
    let Ok(result) = rx.try_recv() else {
        return;
    };
    app.update.job = None;
    let manual = app.update.manual;
    let current = env!("CARGO_PKG_VERSION");
    match result {
        Ok(rel) if is_newer(&rel.version, current) => {
            if manual || rel.version != app.prefs.skipped_version {
                app.update.available = Some(rel);
                app.update.open = true;
            }
        }
        Ok(_) if manual => app.info(tr!(
            "Tienes la versión más reciente ({current}).",
            "You have the latest version ({current})."
        )),
        Err(e) if manual => app.error(tr!(
            "No se pudo buscar actualizaciones: {e}",
            "Could not check for updates: {e}"
        )),
        _ => {}
    }
}

/// "New version available" window.
pub fn window(app: &mut OndaluxApp, ctx: &egui::Context) {
    if !app.update.open {
        return;
    }
    let Some(rel) = app.update.available.clone() else {
        app.update.open = false;
        return;
    };
    let mut open = true;
    let mut action = None;
    egui::Window::new(format!(
        "{}  {}",
        icon::ARROW_CIRCLE_UP,
        t("Nueva versión disponible", "New version available")
    ))
    .id(egui::Id::new("update-window"))
    .open(&mut open)
    .collapsible(false)
    .resizable(false)
    .default_width(420.0)
    .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
    .show(ctx, |ui| {
        let p = style::palette(ui);
        ui.label(
            style::bold(format!("Ondalux Heatmap {}", rel.version))
                .size(17.0)
                .color(p.text),
        );
        let mut meta = tr!("Tienes la {}", "You have {}", env!("CARGO_PKG_VERSION"));
        if !rel.date.is_empty() {
            meta = format!("{} · {meta}", rel.date);
        }
        ui.label(egui::RichText::new(meta).color(p.muted));
        let notes = rel.notes.get();
        if !notes.is_empty() {
            ui.add_space(8.0);
            ui.strong(t("Novedades", "What's new"));
            egui::ScrollArea::vertical()
                .max_height(220.0)
                .show(ui, |ui| {
                    for n in notes {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(egui::RichText::new(icon::CHECK).color(p.success));
                            ui.label(n);
                        });
                    }
                });
        }
        ui.add_space(10.0);
        if style::primary_button(
            ui,
            &format!("{}  {}", icon::DOWNLOAD_SIMPLE, t("Descargar", "Download")),
        )
        .clicked()
        {
            action = Some(0);
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui
                .button(t("Recordarme después", "Remind me later"))
                .clicked()
            {
                action = Some(1);
            }
            if ui
                .button(t("Omitir esta versión", "Skip this version"))
                .clicked()
            {
                action = Some(2);
            }
        });
        ui.add_space(2.0);
        ui.weak(t(
            "Tus proyectos no se modifican. Instala la nueva versión cuando quieras.",
            "Your projects are not changed. Install the new version whenever you like.",
        ));
    });
    match action {
        Some(0) => {
            let url = rel.download.get();
            let url = if url.is_empty() {
                "https://ondaluxheatmap.com/#download"
            } else {
                url.as_str()
            };
            ctx.open_url(egui::OpenUrl::new_tab(url));
            open = false;
        }
        Some(1) => open = false,
        Some(2) => {
            app.prefs.skipped_version = rel.version.clone();
            app.prefs.save();
            open = false;
        }
        _ => {}
    }
    app.update.open = open;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert_eq!(parse_version("v0.2.0"), Some((0, 2, 0)));
        assert_eq!(parse_version("1.4"), Some((1, 4, 0)));
        assert_eq!(parse_version("0.3.0-beta.1"), Some((0, 3, 0)));
        assert!(parse_version("latest").is_none());
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("0.10.0", "0.9.0"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(!is_newer("garbage", "0.2.0"));
    }

    #[test]
    fn reads_version_json() {
        let json = r#"{
            "version": "0.3.0",
            "date": "2026-12-01",
            "download": { "en": "https://ondaluxheatmap.com/#download", "es": "https://ondaluxheatmap.com/es/#download" },
            "notes": { "en": ["Faster"], "es": ["Más rápido"] },
            "file": "ignored.exe"
        }"#;
        let r: Release = serde_json::from_str(json).unwrap();
        assert_eq!(r.version, "0.3.0");
        assert_eq!(r.notes.es, ["Más rápido"]);
        // Only the version is required.
        let r: Release = serde_json::from_str(r#"{"version":"1.0.0"}"#).unwrap();
        assert!(r.notes.en.is_empty());
    }
}
