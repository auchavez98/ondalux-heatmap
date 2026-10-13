//! AP model catalog: model picker for new and existing APs, and the catalog
//! window (browse, search, custom models, import/export).

use crate::app::OndaluxApp;
use eframe::egui::{self, DragValue};
use ondalux_core::ap::ApId;
use ondalux_core::catalog::{ApModel, Catalog, Origin, RadioSpec, TxPowerBasis};
use ondalux_core::channel::{Band, ChannelWidth};
use ondalux_io::user_catalog;
use std::path::PathBuf;

/// Vendor + model name of a catalog entry.
pub type ModelKey = (String, String);

#[derive(Default)]
pub struct CatalogState {
    pub catalog: Catalog,
    /// Where user models are saved, if the OS has a config directory.
    path: Option<PathBuf>,
    /// Model used for APs placed with the AP tool (`None` = generic).
    pub new_ap_model: Option<ModelKey>,
    pub open: bool,
    search: String,
    selected: Option<ModelKey>,
    /// Custom model being edited, and the key it replaces (if any).
    draft: Option<(ApModel, Option<ModelKey>)>,
    draft_error: Option<String>,
}

impl CatalogState {
    /// Built-in catalog plus the user's saved models.
    pub fn load() -> (Self, Option<String>) {
        let mut catalog = Catalog::builtin();
        let path = user_catalog::default_path();
        let mut error = None;
        if let Some(p) = &path {
            match user_catalog::load(p) {
                Ok(models) => catalog.merge_user(models),
                Err(e) => error = Some(format!("Could not read {}: {e}", p.display())),
            }
        }
        (
            Self {
                catalog,
                path,
                ..Default::default()
            },
            error,
        )
    }

    fn save_user(&self) -> Result<(), String> {
        let Some(p) = &self.path else {
            return Err("no configuration folder available to save custom models".into());
        };
        user_catalog::save(p, &self.catalog.user_models()).map_err(|e| e.to_string())
    }

    pub fn new_ap_model(&self) -> Option<&ApModel> {
        let (v, m) = self.new_ap_model.as_ref()?;
        self.catalog.find(v, m)
    }
}

fn key_of(m: &ApModel) -> ModelKey {
    (m.vendor.clone(), m.model.clone())
}

/// A combo box listing "Generic" and every model grouped by vendor.
/// Returns true if the selection changed.
pub fn model_combo(
    ui: &mut egui::Ui,
    id: &str,
    catalog: &Catalog,
    selected: &mut Option<ModelKey>,
    generic_label: &str,
) -> bool {
    let text = selected
        .as_ref()
        .map_or(generic_label.to_owned(), |(v, m)| format!("{v} {m}"));
    let before = selected.clone();
    egui::ComboBox::from_id_salt(id)
        .selected_text(text)
        .width(220.0)
        .height(420.0)
        .show_ui(ui, |ui| {
            ui.selectable_value(selected, None, generic_label);
            for vendor in catalog.vendors() {
                ui.separator();
                ui.weak(&vendor);
                for (m, origin) in catalog.models_of(&vendor) {
                    let label = match origin {
                        Origin::BuiltIn => format!("  {}  ({})", m.model, m.standard),
                        Origin::User => format!("  {}  ({}, custom)", m.model, m.standard),
                    };
                    ui.selectable_value(selected, Some(key_of(m)), label);
                }
            }
        });
    *selected != before
}

/// Model section of the AP inspector: shows the model and lets the user
/// switch it, which replaces the radios with the model's (keeping channels).
pub fn ap_model_editor(app: &mut OndaluxApp, ui: &mut egui::Ui, id: ApId) {
    let Some(doc) = app.doc.as_mut() else {
        return;
    };
    let Some(index) = doc.project.aps.iter().position(|a| a.id == id) else {
        return;
    };
    let ap = &doc.project.aps[index];
    let mut selected = app.catalog.catalog.find(&ap.vendor, &ap.model).map(key_of);
    ui.horizontal(|ui| {
        ui.label("Catalog model");
        if model_combo(
            ui,
            "ap-model",
            &app.catalog.catalog,
            &mut selected,
            "Custom / generic",
        ) {
            if let Some(model) = selected
                .as_ref()
                .and_then(|(v, m)| app.catalog.catalog.find(v, m))
                .cloned()
            {
                doc.checkpoint();
                model.apply_to(&mut doc.project.aps[index], index);
            }
        }
    });
    if let Some(m) = selected
        .as_ref()
        .and_then(|(v, m)| app.catalog.catalog.find(v, m))
    {
        ui.weak(format!(
            "{} · {} · TX power basis: {}",
            m.standard,
            m.bands(),
            m.tx_power_basis.label()
        ));
        if !m.notes.is_empty() {
            ui.weak(&m.notes);
        }
        ui.hyperlink_to("Datasheet source", &m.source);
        ui.weak("Radio values below were filled from the catalog; you can still edit them.");
    }
}

fn new_draft() -> ApModel {
    ApModel {
        vendor: String::new(),
        model: String::new(),
        standard: "Wi-Fi 6".into(),
        environment: "indoor".into(),
        radios: vec![
            RadioSpec {
                band: Band::G2_4,
                max_tx_power_dbm: 20.0,
                antenna_gain_dbi: 4.0,
                spatial_streams: 2,
                max_width: ChannelWidth::Mhz40,
            },
            RadioSpec {
                band: Band::G5,
                max_tx_power_dbm: 23.0,
                antenna_gain_dbi: 5.0,
                spatial_streams: 2,
                max_width: ChannelWidth::Mhz80,
            },
        ],
        tx_power_basis: TxPowerBasis::VendorMax,
        source: "https://".into(),
        notes: String::new(),
    }
}

pub fn window(app: &mut OndaluxApp, ctx: &egui::Context) {
    if !app.catalog.open {
        return;
    }
    let mut open = true;
    let mut status: Option<(String, bool)> = None;
    egui::Window::new("AP catalog")
        .open(&mut open)
        .default_size([760.0, 520.0])
        .show(ctx, |ui| {
            let st = &mut app.catalog;
            ui.horizontal(|ui| {
                ui.label("Search");
                ui.text_edit_singleline(&mut st.search);
                ui.weak(format!(
                    "{} models · {} vendors",
                    st.catalog.len(),
                    st.catalog.vendors().len()
                ));
            });
            ui.separator();
            ui.columns(2, |cols| {
                // ---- list ----
                egui::ScrollArea::vertical()
                    .id_salt("catalog-list")
                    .max_height(420.0)
                    .show(&mut cols[0], |ui| {
                        let mut last_vendor = String::new();
                        for (m, origin) in st.catalog.search(&st.search) {
                            if m.vendor != last_vendor {
                                ui.strong(&m.vendor);
                                last_vendor = m.vendor.clone();
                            }
                            let label = format!(
                                "  {}  · {}{}",
                                m.model,
                                m.standard,
                                if origin == Origin::User { " · custom" } else { "" }
                            );
                            let k = key_of(m);
                            if ui
                                .selectable_label(st.selected.as_ref() == Some(&k), label)
                                .clicked()
                            {
                                st.selected = Some(k);
                                st.draft = None;
                            }
                        }
                    });
                // ---- details / editor ----
                let ui = &mut cols[1];
                if st.draft.is_some() {
                    draft_editor(ui, st, &mut status);
                    return;
                }
                let Some((v, mname)) = st.selected.clone() else {
                    ui.weak("Select a model to see its datasheet values.");
                    return;
                };
                let Some(m) = st.catalog.find(&v, &mname).cloned() else {
                    st.selected = None;
                    return;
                };
                let is_user = st
                    .catalog
                    .user_models()
                    .iter()
                    .any(|u| key_of(u) == key_of(&m));
                ui.heading(format!("{} {}", m.vendor, m.model));
                ui.label(format!("{} · {}", m.standard, m.environment));
                egui::Grid::new("spec-grid").striped(true).show(ui, |ui| {
                    ui.strong("Band");
                    ui.strong("Max TX");
                    ui.strong("Design TX");
                    ui.strong("Gain");
                    ui.strong("MIMO");
                    ui.strong("Max width");
                    ui.end_row();
                    for r in &m.radios {
                        ui.label(r.band.label());
                        ui.label(format!("{:.1} dBm", r.max_tx_power_dbm));
                        ui.label(format!("{:.1} dBm", m.design_tx_power(r).0));
                        ui.label(format!("{:.1} dBi", r.antenna_gain_dbi));
                        ui.label(format!("{0}x{0}", r.spatial_streams));
                        ui.label(format!("{} MHz", r.max_width.mhz()));
                        ui.end_row();
                    }
                });
                ui.weak(format!("Max TX basis: {}.", m.tx_power_basis.label()));
                ui.weak("Design TX = conducted total used for predictions. Regulatory limits in your country may be lower.");
                if !m.notes.is_empty() {
                    ui.label(&m.notes);
                }
                ui.hyperlink_to("Source", &m.source);
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    let is_default = st.new_ap_model.as_ref() == Some(&key_of(&m));
                    if ui
                        .add_enabled(!is_default, egui::Button::new("Use for new APs"))
                        .clicked()
                    {
                        st.new_ap_model = Some(key_of(&m));
                    }
                    if ui.button("Edit as custom…").clicked() {
                        let replaces = is_user.then(|| key_of(&m));
                        st.draft = Some((m.clone(), replaces));
                        st.draft_error = None;
                    }
                    if is_user && ui.button("Delete custom").clicked() {
                        st.catalog.remove_user(&m.vendor, &m.model);
                        status = Some(match st.save_user() {
                            Ok(()) => (format!("Deleted custom model {m}."), false),
                            Err(e) => (e, true),
                        });
                    }
                });
            });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("New custom model…").clicked() {
                    st.draft = Some((new_draft(), None));
                    st.draft_error = None;
                }
                if ui.button("Import JSON…").clicked() {
                    status = Some(import(st));
                }
                if ui.button("Export custom models…").clicked() {
                    status = export(st);
                }
                if let Some(p) = &st.path {
                    ui.weak(format!("Custom models: {}", p.display()));
                }
            });
        });
    app.catalog.open = open;
    if let Some((msg, err)) = status {
        if err {
            app.error(msg);
        } else {
            app.info(msg);
        }
    }
}

fn draft_editor(ui: &mut egui::Ui, st: &mut CatalogState, status: &mut Option<(String, bool)>) {
    let Some((draft, replaces)) = st.draft.as_mut() else {
        return;
    };
    ui.heading("Custom model");
    egui::Grid::new("draft-grid").num_columns(2).show(ui, |ui| {
        ui.label("Vendor");
        ui.text_edit_singleline(&mut draft.vendor);
        ui.end_row();
        ui.label("Model");
        ui.text_edit_singleline(&mut draft.model);
        ui.end_row();
        ui.label("Standard");
        ui.text_edit_singleline(&mut draft.standard);
        ui.end_row();
        ui.label("Environment");
        egui::ComboBox::from_id_salt("draft-env")
            .selected_text(draft.environment.clone())
            .show_ui(ui, |ui| {
                for e in ["indoor", "outdoor"] {
                    ui.selectable_value(&mut draft.environment, e.to_owned(), e);
                }
            });
        ui.end_row();
        ui.label("TX power basis");
        egui::ComboBox::from_id_salt("draft-basis")
            .selected_text(draft.tx_power_basis.label())
            .show_ui(ui, |ui| {
                for b in [
                    TxPowerBasis::VendorMax,
                    TxPowerBasis::AggregateConducted,
                    TxPowerBasis::PerChainConducted,
                    TxPowerBasis::Eirp,
                ] {
                    ui.selectable_value(&mut draft.tx_power_basis, b, b.label());
                }
            });
        ui.end_row();
        ui.label("Source URL");
        ui.text_edit_singleline(&mut draft.source);
        ui.end_row();
        ui.label("Notes");
        ui.text_edit_singleline(&mut draft.notes);
        ui.end_row();
    });
    let mut remove = None;
    egui::Grid::new("draft-radios")
        .striped(true)
        .show(ui, |ui| {
            ui.strong("Band");
            ui.strong("Max TX (dBm)");
            ui.strong("Gain (dBi)");
            ui.strong("Streams");
            ui.strong("Max width");
            ui.label("");
            ui.end_row();
            for (i, r) in draft.radios.iter_mut().enumerate() {
                ui.push_id(i, |ui| {
                    egui::ComboBox::from_id_salt("band")
                        .selected_text(r.band.label())
                        .show_ui(ui, |ui| {
                            for b in Band::ALL {
                                ui.selectable_value(&mut r.band, b, b.label());
                            }
                        });
                });
                ui.add(
                    DragValue::new(&mut r.max_tx_power_dbm)
                        .range(-10.0..=36.0)
                        .speed(0.5),
                );
                ui.add(
                    DragValue::new(&mut r.antenna_gain_dbi)
                        .range(-5.0..=25.0)
                        .speed(0.1),
                );
                ui.add(DragValue::new(&mut r.spatial_streams).range(1..=16));
                ui.push_id(("w", i), |ui| {
                    if !r.band.widths().contains(&r.max_width) {
                        r.max_width = ChannelWidth::Mhz20;
                    }
                    egui::ComboBox::from_id_salt("width")
                        .selected_text(format!("{} MHz", r.max_width.mhz()))
                        .show_ui(ui, |ui| {
                            for w in r.band.widths() {
                                ui.selectable_value(
                                    &mut r.max_width,
                                    *w,
                                    format!("{} MHz", w.mhz()),
                                );
                            }
                        });
                });
                if ui.small_button("Remove").clicked() {
                    remove = Some(i);
                }
                ui.end_row();
            }
        });
    if let Some(i) = remove {
        draft.radios.remove(i);
    }
    if ui.button("Add radio").clicked() {
        draft.radios.push(RadioSpec {
            band: Band::G6,
            max_tx_power_dbm: 23.0,
            antenna_gain_dbi: 5.0,
            spatial_streams: 2,
            max_width: ChannelWidth::Mhz160,
        });
    }
    let draft_copy = draft.clone();
    let replaces = replaces.clone();
    if let Some(e) = &st.draft_error {
        ui.colored_label(ui.visuals().error_fg_color, e);
    }
    let (save, cancel) = ui
        .horizontal(|ui| (ui.button("Save").clicked(), ui.button("Cancel").clicked()))
        .inner;
    if cancel {
        st.draft = None;
        st.draft_error = None;
    } else if save {
        match draft_copy.validate() {
            Ok(()) => {
                let model = draft_copy;
                if let Some((v, m)) = replaces {
                    if (v.as_str(), m.as_str()) != (model.vendor.as_str(), model.model.as_str()) {
                        st.catalog.remove_user(&v, &m);
                    }
                }
                st.selected = Some(key_of(&model));
                st.catalog.upsert_user(model.clone());
                st.draft = None;
                st.draft_error = None;
                *status = Some(match st.save_user() {
                    Ok(()) => (format!("Saved custom model {model}."), false),
                    Err(e) => (e, true),
                });
            }
            Err(e) => st.draft_error = Some(e),
        }
    }
}
fn import(st: &mut CatalogState) -> (String, bool) {
    let Some(path) = rfd::FileDialog::new()
        .set_title("Import AP models")
        .add_filter("JSON", &["json"])
        .pick_file()
    else {
        return ("Import cancelled.".into(), false);
    };
    let result = std::fs::read_to_string(&path)
        .map_err(|e| e.to_string())
        .and_then(|t| user_catalog::parse(&t).map_err(|e| e.to_string()));
    match result {
        Ok(models) => {
            let n = models.len();
            st.catalog.merge_user(models);
            match st.save_user() {
                Ok(()) => (
                    format!("Imported {n} models from {}.", path.display()),
                    false,
                ),
                Err(e) => (
                    format!("Imported {n} models but could not save them: {e}"),
                    true,
                ),
            }
        }
        Err(e) => (format!("Could not import {}: {e}", path.display()), true),
    }
}

fn export(st: &CatalogState) -> Option<(String, bool)> {
    let models = st.catalog.user_models();
    if models.is_empty() {
        return Some(("There are no custom models to export.".into(), true));
    }
    let path = rfd::FileDialog::new()
        .set_title("Export custom AP models")
        .set_file_name("ap-models.json")
        .add_filter("JSON", &["json"])
        .save_file()?;
    Some(match user_catalog::save(&path, &models) {
        Ok(()) => (format!("Exported {} models.", models.len()), false),
        Err(e) => (format!("Could not export: {e}"), true),
    })
}
