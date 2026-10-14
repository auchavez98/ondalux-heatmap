//! Application state, menus, shortcuts and layout.

use crate::canvas::CanvasState;
use crate::catalog_ui::{self, CatalogState};
use crate::document::Document;
use crate::export_ui::{self, ExportState};
use crate::heatmap::{self, Comparison, Field, Source, Worker};
use crate::i18n::{self, t, ThemeChoice, UiPrefs};
use crate::panels;
use crate::style;
use crate::survey_ui::{self, SurveyState};
use crate::tr;
use crate::view::View;
use eframe::egui::{self, Key, KeyboardShortcut, Modifiers, TextureHandle, TextureOptions};
use ondalux_core::ap::ApId;
use ondalux_core::channel::Band;
use ondalux_core::grid::GridSpec;
use ondalux_core::interpolate::Variogram;
use ondalux_core::material::MaterialId;
use ondalux_core::metrics::MetricKind;
use ondalux_core::project::Project;
use ondalux_io::{decode_plan, import_plan, ProjectFile};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Select,
    Wall,
    Ap,
    Calibrate,
    Survey,
}

impl Tool {
    pub const ALL: [Tool; 5] = [
        Tool::Select,
        Tool::Wall,
        Tool::Ap,
        Tool::Calibrate,
        Tool::Survey,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => t("Seleccionar", "Select"),
            Tool::Wall => t("Paredes", "Walls"),
            Tool::Ap => t("Puntos de acceso", "Access points"),
            Tool::Calibrate => t("Escala", "Scale"),
            Tool::Survey => t("Medir WiFi", "Measure WiFi"),
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Tool::Select => "↖",
            Tool::Wall => "✏",
            Tool::Ap => "📡",
            Tool::Calibrate => "↔",
            Tool::Survey => "📶",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Tool::Select => "V",
            Tool::Wall => "W",
            Tool::Ap => "A",
            Tool::Calibrate => "C",
            Tool::Survey => "S",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Tool::Select => t(
                "Clic para seleccionar · arrastra un AP o el extremo de una pared para moverlo · arrastra el fondo para desplazar · rueda para zoom",
                "Click to select · drag an AP or a wall end to move it · drag the background to pan · wheel to zoom",
            ),
            Tool::Wall => t(
                "Clic para agregar puntos de pared · Mayús = 45° · Esc, clic derecho o doble clic termina",
                "Click to add wall points · Shift = 45° · Esc, right-click or double-click ends",
            ),
            Tool::Ap => t(
                "Clic para colocar un punto de acceso · Esc vuelve a Seleccionar",
                "Click to place an access point · Esc returns to Select",
            ),
            Tool::Calibrate => t(
                "Clic en dos puntos de distancia conocida y escribe la distancia",
                "Click two points a known distance apart, then type the distance",
            ),
            Tool::Survey => t(
                "Quédate quieto y haz clic en tu posición para registrar un escaneo WiFi · Esc vuelve a Seleccionar",
                "Stand still and click your position to record a WiFi scan · Esc returns to Select",
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    None,
    Ap(ApId),
    Wall(usize),
    /// A point of the active survey, by measurement id.
    Measurement(u32),
}

/// Content of the right side panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RightTab {
    Inspector,
    Survey,
}

/// Action waiting for the user to confirm discarding unsaved changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    NewFromPlan,
    Open,
    Close,
    Quit,
}

pub struct HeatmapView {
    pub show: bool,
    pub metric: MetricKind,
    pub band: Band,
    pub opacity: f32,
    pub source: Source,
    /// Difference view: hide cells with |Δ| below this (dB).
    pub diff_threshold: f32,
    pub texture: Option<TextureHandle>,
    pub field: Option<Arc<Field>>,
    pub comparison: Option<Comparison>,
    pub measured_radios: usize,
    pub variogram: Option<Variogram>,
    pub last_elapsed: Option<Duration>,
    pub last_recomputed: usize,
}

pub struct OndaluxApp {
    pub doc: Option<Document>,
    pub tool: Tool,
    pub selection: Selection,
    pub view: View,
    pub fit_pending: bool,
    pub plan_texture: Option<TextureHandle>,
    pub canvas: CanvasState,
    pub wall_material: MaterialId,
    pub heat: HeatmapView,
    worker: Worker,
    pub survey: SurveyState,
    pub right_tab: RightTab,
    pub catalog: CatalogState,
    pub export: ExportState,
    pub show_materials: bool,
    pub show_settings: bool,
    show_about: bool,
    pending: Option<Pending>,
    allow_quit: bool,
    /// Message shown in the status bar and whether it is an error.
    pub status: Option<(String, bool)>,
    status_at: Option<Instant>,
    pub prefs: UiPrefs,
    show_shortcuts: bool,
}

/// How long status bar messages stay visible.
const STATUS_TTL: Duration = Duration::from_secs(8);

impl OndaluxApp {
    /// `path` (from the command line) may be a project or a floor plan image.
    pub fn new(cc: &eframe::CreationContext<'_>, path: Option<PathBuf>) -> Self {
        let prefs = UiPrefs::load();
        i18n::set_english(prefs.english);
        style::apply(&cc.egui_ctx, prefs.theme);
        let (catalog, catalog_error) = CatalogState::load();
        let mut app = Self {
            doc: None,
            tool: Tool::Select,
            selection: Selection::None,
            view: View::default(),
            fit_pending: false,
            plan_texture: None,
            canvas: CanvasState::default(),
            wall_material: MaterialId(1),
            heat: HeatmapView {
                show: true,
                metric: MetricKind::Rssi,
                band: Band::G5,
                opacity: 0.6,
                source: Source::Predicted,
                diff_threshold: 0.0,
                texture: None,
                field: None,
                comparison: None,
                measured_radios: 0,
                variogram: None,
                last_elapsed: None,
                last_recomputed: 0,
            },
            worker: Worker::spawn(cc.egui_ctx.clone()),
            survey: SurveyState::default(),
            right_tab: RightTab::Inspector,
            catalog,
            export: ExportState::default(),
            show_materials: false,
            show_settings: false,
            show_about: false,
            pending: None,
            allow_quit: false,
            status: None,
            status_at: None,
            prefs,
            show_shortcuts: false,
        };
        if let Some(e) = catalog_error {
            app.error(e);
        }
        if let Some(path) = path {
            let is_project = path.extension().is_some_and(|e| {
                e.eq_ignore_ascii_case(ondalux_io::project_file::EXTENSION)
                    || e.eq_ignore_ascii_case(ondalux_io::project_file::LEGACY_EXTENSION)
            });
            if is_project {
                app.open_path(&cc.egui_ctx, path);
            } else {
                app.import_plan_path(&cc.egui_ctx, path);
            }
        }
        app
    }

    /// True once a heat map has been computed and none is pending.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn heat_ready(&self) -> bool {
        self.heat.field.is_some() && !self.worker.busy()
    }

    pub fn info(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), false));
        self.status_at = Some(Instant::now());
    }

    pub fn error(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), true));
        self.status_at = Some(Instant::now());
    }

    fn is_dirty(&self) -> bool {
        self.doc.as_ref().is_some_and(|d| d.dirty)
    }

    /// Runs `action` now, or asks first if there are unsaved changes.
    fn guarded(&mut self, ctx: &egui::Context, action: Pending) {
        if self.is_dirty() {
            self.pending = Some(action);
        } else {
            self.run_pending(ctx, action);
        }
    }

    fn run_pending(&mut self, ctx: &egui::Context, action: Pending) {
        match action {
            Pending::NewFromPlan => self.new_from_plan(ctx),
            Pending::Open => self.open(ctx),
            Pending::Close => self.set_document(ctx, None),
            Pending::Quit => {
                self.allow_quit = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn set_document(&mut self, ctx: &egui::Context, doc: Option<Document>) {
        self.plan_texture = doc.as_ref().and_then(|d| {
            let plan = d.plan_image.as_ref()?;
            match decode_plan(plan) {
                Ok(img) => {
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [img.width as usize, img.height as usize],
                        &img.rgba,
                    );
                    Some(ctx.load_texture("floor-plan", image, TextureOptions::LINEAR))
                }
                Err(e) => {
                    self.status = Some((
                        tr!(
                            "No se pudo leer el plano: {e}",
                            "Could not decode floor plan: {e}"
                        ),
                        true,
                    ));
                    None
                }
            }
        });
        self.doc = doc;
        self.selection = Selection::None;
        self.canvas = CanvasState::default();
        self.heat.texture = None;
        self.heat.field = None;
        self.heat.comparison = None;
        self.survey.active = 0;
        self.worker.reset();
        self.fit_pending = true;
        self.wall_material = self
            .doc
            .as_ref()
            .and_then(|d| d.project.materials.iter().next().map(|m| m.id))
            .unwrap_or(MaterialId(1));
    }

    fn new_from_plan(&mut self, ctx: &egui::Context) {
        let Some(path) = rfd::FileDialog::new()
            .set_title(t("Importar plano", "Import floor plan"))
            .add_filter(t("Planos", "Floor plans"), &["png", "jpg", "jpeg", "pdf"])
            .pick_file()
        else {
            return;
        };
        self.import_plan_path(ctx, path);
    }

    fn import_plan_path(&mut self, ctx: &egui::Context, path: PathBuf) {
        match import_plan(&path) {
            Ok((plan, decoded)) => {
                let name = path
                    .file_stem()
                    .map_or_else(|| "Untitled".into(), |s| s.to_string_lossy().into_owned());
                let project = Project::new(name, decoded.width, decoded.height);
                let mut doc = Document::new(project, Some(plan), None);
                doc.dirty = true;
                self.set_document(ctx, Some(doc));
                self.tool = Tool::Calibrate;
                self.info(t("Plano importado. Calibra la escala: haz clic en dos puntos de distancia conocida.", "Floor plan imported. Calibrate the scale: click two points a known distance apart."));
            }
            Err(e) => self.error(tr!(
                "No se pudo importar {}: {e}",
                "Could not import {}: {e}",
                path.display()
            )),
        }
    }

    fn open(&mut self, ctx: &egui::Context) {
        let Some(path) = rfd::FileDialog::new()
            .set_title(t("Abrir proyecto", "Open project"))
            .add_filter(
                "Ondalux Heatmap project",
                &[
                    ondalux_io::project_file::EXTENSION,
                    ondalux_io::project_file::LEGACY_EXTENSION,
                ],
            )
            .pick_file()
        else {
            return;
        };
        self.open_path(ctx, path);
    }

    fn open_path(&mut self, ctx: &egui::Context, path: PathBuf) {
        match ProjectFile::load(&path) {
            Ok(file) => {
                self.set_document(ctx, Some(Document::from_file(file, path.clone())));
                self.info(tr!("Abierto {}", "Opened {}", file_name(&path)));
            }
            Err(e) => self.error(tr!(
                "No se pudo abrir {}: {e}",
                "Could not open {}: {e}",
                path.display()
            )),
        }
    }

    fn save(&mut self, save_as: bool) {
        let Some(doc) = self.doc.as_mut() else {
            return;
        };
        let path = match (&doc.path, save_as) {
            (Some(p), false) => p.clone(),
            _ => {
                let Some(mut p) = rfd::FileDialog::new()
                    .set_title(t("Guardar proyecto", "Save project"))
                    .set_file_name(format!(
                        "{}.{}",
                        doc.project.name,
                        ondalux_io::project_file::EXTENSION
                    ))
                    .add_filter(
                        "Ondalux Heatmap project",
                        &[ondalux_io::project_file::EXTENSION],
                    )
                    .save_file()
                else {
                    return;
                };
                if p.extension().is_none() {
                    p.set_extension(ondalux_io::project_file::EXTENSION);
                }
                p
            }
        };
        match doc.to_file().save(&path) {
            Ok(()) => {
                doc.path = Some(path.clone());
                doc.dirty = false;
                self.info(tr!("Guardado {}", "Saved {}", file_name(&path)));
            }
            Err(e) => self.error(tr!(
                "No se pudo guardar {}: {e}",
                "Could not save {}: {e}",
                path.display()
            )),
        }
    }

    pub fn undo(&mut self) {
        if let Some(doc) = &mut self.doc {
            if doc.undo() {
                self.selection = Selection::None;
            }
        }
    }

    pub fn redo(&mut self) {
        if let Some(doc) = &mut self.doc {
            if doc.redo() {
                self.selection = Selection::None;
            }
        }
    }

    pub fn delete_selection(&mut self) {
        let Some(doc) = &mut self.doc else {
            return;
        };
        match self.selection {
            Selection::Ap(id) => {
                doc.checkpoint();
                doc.project.aps.retain(|a| a.id != id);
            }
            Selection::Wall(i) if i < doc.project.walls.len() => {
                doc.checkpoint();
                doc.project.walls.remove(i);
            }
            Selection::Measurement(id) => {
                doc.checkpoint();
                if let Some(s) = doc.project.surveys.get_mut(self.survey.active) {
                    s.measurements.retain(|m| m.id != id);
                }
            }
            _ => return,
        }
        self.selection = Selection::None;
    }

    pub fn set_tool(&mut self, tool: Tool) {
        self.tool = tool;
        self.canvas.cancel();
        if tool == Tool::Survey {
            self.right_tab = RightTab::Survey;
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let cmd = |key| KeyboardShortcut::new(Modifiers::COMMAND, key);
        let cmd_shift = |key| KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, key);
        let pressed = |s: KeyboardShortcut| ctx.input_mut(|i| i.consume_shortcut(&s));

        if pressed(cmd_shift(Key::S)) {
            self.save(true);
        } else if pressed(cmd(Key::S)) {
            self.save(false);
        }
        if pressed(cmd(Key::N)) {
            self.guarded(ctx, Pending::NewFromPlan);
        }
        if pressed(cmd(Key::O)) {
            self.guarded(ctx, Pending::Open);
        }
        if pressed(cmd_shift(Key::Z)) || pressed(cmd(Key::Y)) {
            self.redo();
        } else if pressed(cmd(Key::Z)) {
            self.undo();
        }

        // Single-key shortcuts only when no text field has focus.
        if ctx.egui_wants_keyboard_input() || self.doc.is_none() {
            return;
        }
        let key = |k| ctx.input(|i| i.key_pressed(k) && i.modifiers.is_none());
        if key(Key::V) {
            self.set_tool(Tool::Select);
        }
        if key(Key::W) {
            self.set_tool(Tool::Wall);
        }
        if key(Key::A) {
            self.set_tool(Tool::Ap);
        }
        if key(Key::C) {
            self.set_tool(Tool::Calibrate);
        }
        if key(Key::S) {
            self.set_tool(Tool::Survey);
        }
        if key(Key::F) {
            self.fit_pending = true;
        }
        if key(Key::H) {
            self.heat.show = !self.heat.show;
        }
        if key(Key::Delete) || key(Key::Backspace) {
            self.delete_selection();
        }
        if key(Key::Escape) {
            if self.canvas.is_drafting() {
                self.canvas.cancel();
            } else if self.tool != Tool::Select {
                self.set_tool(Tool::Select);
            } else {
                self.selection = Selection::None;
            }
        }
        let digits = [
            Key::Num1,
            Key::Num2,
            Key::Num3,
            Key::Num4,
            Key::Num5,
            Key::Num6,
        ];
        for (k, m) in digits.into_iter().zip(MetricKind::ALL) {
            if key(k) {
                self.heat.metric = m;
                self.heat.show = true;
            }
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let short = |m: Modifiers, k: Key| ctx.format_shortcut(&KeyboardShortcut::new(m, k));
        let has_doc = self.doc.is_some();
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button(t("Archivo", "File"), |ui| {
                if ui
                    .add(
                        egui::Button::new(t("Nuevo desde plano…", "New from floor plan…"))
                            .shortcut_text(short(Modifiers::COMMAND, Key::N)),
                    )
                    .clicked()
                {
                    self.guarded(&ctx, Pending::NewFromPlan);
                }
                if ui
                    .add(
                        egui::Button::new(t("Abrir…", "Open…"))
                            .shortcut_text(short(Modifiers::COMMAND, Key::O)),
                    )
                    .clicked()
                {
                    self.guarded(&ctx, Pending::Open);
                }
                ui.separator();
                if ui
                    .add_enabled(
                        has_doc,
                        egui::Button::new(t("Guardar", "Save"))
                            .shortcut_text(short(Modifiers::COMMAND, Key::S)),
                    )
                    .clicked()
                {
                    self.save(false);
                }
                if ui
                    .add_enabled(
                        has_doc,
                        egui::Button::new(t("Guardar como…", "Save as…"))
                            .shortcut_text(short(Modifiers::COMMAND | Modifiers::SHIFT, Key::S)),
                    )
                    .clicked()
                {
                    self.save(true);
                }
                ui.separator();
                ui.add_enabled_ui(has_doc, |ui| {
                    ui.menu_button(t("Exportar", "Export"), |ui| {
                        if ui.button(t("Reporte PDF…", "PDF report…")).clicked() {
                            self.export.report_open = true;
                        }
                        if ui
                            .button(t("Imagen del mapa (PNG)…", "Heat map image (PNG)…"))
                            .clicked()
                        {
                            export_ui::export_png(self);
                        }
                        if ui
                            .button(t("Lista de APs (CSV)…", "Access point list (CSV)…"))
                            .clicked()
                        {
                            export_ui::export_ap_csv(self);
                        }
                        if ui
                            .button(t("Mediciones (CSV)…", "Survey measurements (CSV)…"))
                            .clicked()
                        {
                            survey_ui::export_csv(self);
                        }
                    });
                });
                ui.separator();
                if ui
                    .add_enabled(
                        has_doc,
                        egui::Button::new(t("Cerrar proyecto", "Close project")),
                    )
                    .clicked()
                {
                    self.guarded(&ctx, Pending::Close);
                }
                if ui.button(t("Salir", "Quit")).clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button(t("Editar", "Edit"), |ui| {
                let (can_undo, can_redo) = self
                    .doc
                    .as_ref()
                    .map_or((false, false), |d| (d.can_undo(), d.can_redo()));
                if ui
                    .add_enabled(
                        can_undo,
                        egui::Button::new(t("Deshacer", "Undo"))
                            .shortcut_text(short(Modifiers::COMMAND, Key::Z)),
                    )
                    .clicked()
                {
                    self.undo();
                }
                if ui
                    .add_enabled(
                        can_redo,
                        egui::Button::new(t("Rehacer", "Redo"))
                            .shortcut_text(short(Modifiers::COMMAND, Key::Y)),
                    )
                    .clicked()
                {
                    self.redo();
                }
                ui.separator();
                if ui
                    .add_enabled(
                        self.selection != Selection::None,
                        egui::Button::new(t("Eliminar selección", "Delete selection"))
                            .shortcut_text("Supr"),
                    )
                    .clicked()
                {
                    self.delete_selection();
                }
                ui.separator();
                if ui
                    .add_enabled(
                        has_doc,
                        egui::Button::new(t("Materiales de pared…", "Wall materials…")),
                    )
                    .clicked()
                {
                    self.show_materials = true;
                }
                if ui.button(t("Catálogo de APs…", "AP catalog…")).clicked() {
                    self.catalog.open = true;
                }
                if ui
                    .add_enabled(
                        has_doc,
                        egui::Button::new(t(
                            "Requisitos y modelo de predicción…",
                            "Requirements and prediction model…",
                        )),
                    )
                    .clicked()
                {
                    self.show_settings = true;
                }
            });
            ui.menu_button(t("Ver", "View"), |ui| {
                if ui
                    .add(
                        egui::Button::new(t("Ajustar plano a la ventana", "Fit plan to window"))
                            .shortcut_text("F"),
                    )
                    .clicked()
                {
                    self.fit_pending = true;
                }
                ui.checkbox(
                    &mut self.heat.show,
                    t("Mostrar mapa de calor (H)", "Show heat map (H)"),
                );
                ui.separator();
                let mut prefs = self.prefs.clone();
                ui.menu_button(t("Idioma", "Language"), |ui| {
                    ui.radio_value(&mut prefs.english, false, "Español");
                    ui.radio_value(&mut prefs.english, true, "English");
                });
                ui.menu_button(t("Tema", "Theme"), |ui| {
                    ui.radio_value(
                        &mut prefs.theme,
                        ThemeChoice::System,
                        t("Como el sistema", "Follow system"),
                    );
                    ui.radio_value(&mut prefs.theme, ThemeChoice::Light, t("Claro", "Light"));
                    ui.radio_value(&mut prefs.theme, ThemeChoice::Dark, t("Oscuro", "Dark"));
                });
                if prefs != self.prefs {
                    i18n::set_english(prefs.english);
                    style::apply(&ctx, prefs.theme);
                    prefs.save();
                    self.prefs = prefs;
                }
            });
            ui.menu_button(t("Ayuda", "Help"), |ui| {
                if ui
                    .button(t("Atajos de teclado", "Keyboard shortcuts"))
                    .clicked()
                {
                    self.show_shortcuts = true;
                }
                if ui
                    .button(t("Acerca de Ondalux Heatmap", "About Ondalux Heatmap"))
                    .clicked()
                {
                    self.show_about = true;
                }
            });
        });
    }

    /// Left column: tools, then the options of the active tool.
    fn tool_panel(&mut self, ui: &mut egui::Ui) {
        style::section(ui, t("Herramientas", "Tools"));
        for tool in Tool::ALL {
            let text = format!("{}   {}", tool.icon(), tool.label());
            let r = ui
                .add_sized(
                    [ui.available_width(), 32.0],
                    egui::Button::new(text).selected(self.tool == tool),
                )
                .on_hover_text(format!(
                    "{}\n\n{}: {}",
                    tool.hint(),
                    t("Atajo", "Shortcut"),
                    tool.key()
                ));
            if r.clicked() {
                self.set_tool(tool);
            }
        }
        ui.add_space(4.0);
        ui.separator();
        style::section(ui, t("Opciones", "Options"));
        let Some(doc) = self.doc.as_ref() else {
            return;
        };
        match self.tool {
            Tool::Select => {
                ui.label(t(
                    "Haz clic en un AP, pared o punto medido para ver y editar sus datos. Arrastra para moverlo.",
                    "Click an AP, wall or survey point to see and edit it. Drag to move it.",
                ));
            }
            Tool::Wall => {
                ui.label(t("Material", "Material"));
                let current = doc
                    .project
                    .materials
                    .get(self.wall_material)
                    .map_or("—".to_owned(), |m| m.name.clone());
                egui::ComboBox::from_id_salt("wall-material")
                    .selected_text(current)
                    .width(ui.available_width())
                    .show_ui(ui, |ui| {
                        for m in doc.project.materials.iter() {
                            ui.selectable_value(&mut self.wall_material, m.id, &m.name);
                        }
                    });
                ui.weak(t(
                    "Clic para agregar puntos. Mayús = ángulos de 45°. Esc o doble clic termina la pared.",
                    "Click to add points. Shift = 45° angles. Esc or double-click ends the wall.",
                ));
                if ui
                    .button(t("Editar materiales…", "Edit materials…"))
                    .clicked()
                {
                    self.show_materials = true;
                }
            }
            Tool::Ap => {
                ui.label(t("Modelo para nuevos APs", "Model for new APs"));
                catalog_ui::model_combo(
                    ui,
                    "new-ap-model",
                    &self.catalog.catalog,
                    &mut self.catalog.new_ap_model,
                    t("Genérico", "Generic"),
                );
                ui.weak(t(
                    "Clic en el plano para colocar un AP. Sus radios se llenan con los datos del modelo.",
                    "Click on the plan to place an AP. Its radios are filled from the model.",
                ));
                if ui.button(t("Ver catálogo…", "Browse catalog…")).clicked() {
                    self.catalog.open = true;
                }
            }
            Tool::Calibrate => {
                match doc.project.plan.calibration {
                    Some(c) => ui.label(format!(
                        "{} {:.1} px/m",
                        t("Escala actual:", "Current scale:"),
                        c.px_per_m()
                    )),
                    None => ui.colored_label(
                        ui.visuals().warn_fg_color,
                        t("Escala sin calibrar", "Scale not calibrated"),
                    ),
                };
                ui.weak(t(
                    "Haz clic en dos puntos cuya distancia real conozcas (una puerta, una cota) y escribe la distancia.",
                    "Click two points whose real distance you know (a door, a dimension) and type the distance.",
                ));
            }
            Tool::Survey => {
                ui.weak(t(
                    "Párate en un lugar, haz clic en tu posición en el plano y espera a que termine el escaneo.",
                    "Stand somewhere, click your position on the plan and wait for the scan to finish.",
                ));
                if ui
                    .button(t("Abrir panel de mediciones", "Open survey panel"))
                    .clicked()
                {
                    self.right_tab = RightTab::Survey;
                }
            }
        }
        ui.add_space(8.0);
        if ui
            .button(t("⛶  Ajustar vista", "⛶  Fit view"))
            .on_hover_text("F")
            .clicked()
        {
            self.fit_pending = true;
        }
    }

    /// Bar above the plan: what the heat map shows.
    fn view_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.heat.show, t("Mapa de calor", "Heat map"))
                .on_hover_text("H");
            ui.add_enabled_ui(self.heat.show, |ui| {
                ui.separator();
                ui.label(t("Mostrar", "Show"));
                egui::ComboBox::from_id_salt("heatmap-metric")
                    .selected_text(metric_label(self.heat.metric))
                    .width(190.0)
                    .show_ui(ui, |ui| {
                        for (i, m) in MetricKind::ALL.into_iter().enumerate() {
                            ui.selectable_value(&mut self.heat.metric, m, metric_label(m))
                                .on_hover_text(format!(
                                    "{}\n\n{}: {}",
                                    metric_help(m),
                                    t("Atajo", "Shortcut"),
                                    i + 1
                                ));
                        }
                    })
                    .response
                    .on_hover_text(metric_help(self.heat.metric));
                ui.label(t("Banda", "Band"));
                egui::ComboBox::from_id_salt("heatmap-band")
                    .selected_text(self.heat.band.label())
                    .width(90.0)
                    .show_ui(ui, |ui| {
                        for b in Band::ALL {
                            ui.selectable_value(&mut self.heat.band, b, b.label());
                        }
                    });
                ui.label(t("Datos", "Data"));
                egui::ComboBox::from_id_salt("heatmap-source")
                    .selected_text(source_label(self.heat.source))
                    .width(150.0)
                    .show_ui(ui, |ui| {
                        for s in Source::ALL {
                            ui.selectable_value(&mut self.heat.source, s, source_label(s))
                                .on_hover_text(source_help(s));
                        }
                    })
                    .response
                    .on_hover_text(source_help(self.heat.source));
                ui.label(t("Opacidad", "Opacity"));
                ui.add(egui::Slider::new(&mut self.heat.opacity, 0.1..=1.0).show_value(false));
            });
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        if self.status_at.is_some_and(|t| t.elapsed() > STATUS_TTL) {
            self.status = None;
            self.status_at = None;
        }
        // Right-to-left so the readouts keep their space and the message
        // on the left is truncated instead of overlapping them.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if self.worker.busy() {
                ui.spinner();
            }
            if let Some(c) = self.heat.field.as_ref().and_then(|f| f.coverage) {
                let req = self.doc.as_ref().map(|d| d.project.requirements);
                let tip = req.map_or(String::new(), |r| {
                    format!(
                        "{} ≥ {:.0} dBm, SNR ≥ {:.0} dB",
                        t("Área que cumple: señal", "Area meeting: signal"),
                        r.min_rssi.0,
                        r.min_snr.0
                    )
                });
                ui.label(format!("{} {:.0} %", t("Cobertura", "Coverage"), c * 100.0))
                    .on_hover_text(tip);
                ui.separator();
            }
            if let Some(text) = self.canvas.hover_text.take() {
                ui.weak(text);
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let (text, color) = match &self.status {
                    Some((msg, true)) => (msg.as_str(), ui.visuals().error_fg_color),
                    Some((msg, false)) => (msg.as_str(), ui.visuals().text_color()),
                    None => (self.tool.hint(), ui.visuals().weak_text_color()),
                };
                ui.add(egui::Label::new(egui::RichText::new(text).color(color)).truncate());
            });
        });
    }

    fn request_heatmap(&mut self) {
        let Some(doc) = &self.doc else {
            return;
        };
        let p = &doc.project;
        let (w, h) = p.plan.size_m();
        let inputs = heatmap::Inputs {
            walls: p.walls.clone(),
            materials: p.materials.clone(),
            settings: p.settings.clone(),
            aps: p.aps.clone(),
            requirements: p.requirements,
            grid: GridSpec::covering(w, h, p.grid_cell_m.clamp(0.05, 5.0)),
            band: self.heat.band,
            metric: self.heat.metric,
            source: self.heat.source,
            survey: p.surveys.get(self.survey.active).cloned(),
            measured: p.survey_settings.clone(),
            diff_threshold: self.heat.diff_threshold,
        };
        self.worker.request(inputs);
    }

    fn poll_heatmap(&mut self, ctx: &egui::Context) {
        if let Some(out) = self.worker.poll() {
            match &mut self.heat.texture {
                Some(t) => t.set(out.image, TextureOptions::LINEAR),
                None => {
                    self.heat.texture =
                        Some(ctx.load_texture("heatmap", out.image, TextureOptions::LINEAR));
                }
            }
            self.heat.field = Some(out.field);
            self.heat.comparison = out.comparison;
            self.heat.measured_radios = out.measured_radios;
            self.heat.variogram = out.variogram;
            self.heat.last_elapsed = Some(out.elapsed);
            self.heat.last_recomputed = out.layers_recomputed;
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(action) = self.pending {
            let mut choice = None;
            egui::Modal::new(egui::Id::new("unsaved")).show(ctx, |ui| {
                ui.heading(t("Cambios sin guardar", "Unsaved changes"));
                ui.label(t(
                    "El proyecto tiene cambios sin guardar.",
                    "The project has unsaved changes.",
                ));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(t("Guardar", "Save")).clicked() {
                        choice = Some(0);
                    }
                    if ui.button(t("Descartar", "Discard")).clicked() {
                        choice = Some(1);
                    }
                    if ui.button(t("Cancelar", "Cancel")).clicked() {
                        choice = Some(2);
                    }
                });
            });
            match choice {
                Some(0) => {
                    self.save(false);
                    self.pending = None;
                    if !self.is_dirty() {
                        self.run_pending(ctx, action);
                    }
                }
                Some(1) => {
                    self.pending = None;
                    if let Some(d) = &mut self.doc {
                        d.dirty = false;
                    }
                    self.run_pending(ctx, action);
                }
                Some(_) => self.pending = None,
                None => {}
            }
        }

        if self.show_about {
            egui::Window::new(t("Acerca de Ondalux Heatmap", "About Ondalux Heatmap"))
                .open(&mut self.show_about)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(format!("Ondalux Heatmap {}", env!("CARGO_PKG_VERSION")));
                    ui.label(t(
                        "Herramienta de código abierto para diseño WiFi y mapas de calor.",
                        "Open source WiFi design and heat map tool.",
                    ));
                    ui.label(t(
                        "Licencia MIT o Apache-2.0.",
                        "Licensed under MIT or Apache-2.0.",
                    ));
                });
        }

        if self.show_shortcuts {
            shortcuts_window(ctx, &mut self.show_shortcuts);
        }
        panels::calibration_dialog(self, ctx);
        panels::materials_window(self, ctx);
        panels::settings_window(self, ctx);
        catalog_ui::window(self, ctx);
        export_ui::report_window(self, ctx);
    }
}

impl eframe::App for OndaluxApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_quit && self.is_dirty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending = Some(Pending::Quit);
        }

        let title = match &self.doc {
            Some(d) => format!("{} — Ondalux Heatmap", d.title()),
            None => "Ondalux Heatmap".to_owned(),
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));

        self.shortcuts(&ctx);
        self.poll_heatmap(&ctx);
        survey_ui::poll(self);
        export_ui::poll(self);

        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        if self.doc.is_some() {
            egui::Panel::left("tools")
                .resizable(false)
                .exact_size(200.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.tool_panel(ui));
                });
            egui::Panel::right("inspector")
                .resizable(true)
                .default_size(320.0)
                .show(ui, |ui| {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.selectable_value(
                            &mut self.right_tab,
                            RightTab::Inspector,
                            egui::RichText::new(t("Propiedades", "Properties")).strong(),
                        );
                        ui.selectable_value(
                            &mut self.right_tab,
                            RightTab::Survey,
                            egui::RichText::new(t("Mediciones", "Survey")).strong(),
                        );
                    });
                    ui.separator();
                    egui::ScrollArea::vertical().show(ui, |ui| match self.right_tab {
                        RightTab::Inspector => panels::inspector(self, ui),
                        RightTab::Survey => survey_ui::panel(self, ui),
                    });
                });
            egui::Panel::top("view").show(ui, |ui| self.view_bar(ui));
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(ui.visuals().extreme_bg_color))
            .show(ui, |ui| {
                if self.doc.is_some() {
                    crate::canvas::show(self, ui);
                } else {
                    welcome(self, ui);
                }
            });

        self.dialogs(&ctx);
        self.request_heatmap();
    }
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

fn welcome(app: &mut OndaluxApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.14).max(20.0));
        ui.label(egui::RichText::new("Ondalux Heatmap").size(34.0).strong());
        ui.label(
            egui::RichText::new(t(
                "Diseño y medición de redes WiFi con mapas de calor",
                "WiFi network design and survey with heat maps",
            ))
            .size(17.0)
            .weak(),
        );
        ui.add_space(26.0);
        if style::primary_button(
            ui,
            t(
                "Nuevo proyecto desde un plano…",
                "New project from a floor plan…",
            ),
        )
        .clicked()
        {
            app.guarded(&ctx, Pending::NewFromPlan);
        }
        ui.add_space(4.0);
        if ui
            .add_sized(
                [ui.available_width().min(320.0), 32.0],
                egui::Button::new(t("Abrir proyecto…", "Open project…")),
            )
            .clicked()
        {
            app.guarded(&ctx, Pending::Open);
        }
        ui.add_space(34.0);
        ui.label(egui::RichText::new(t("Cómo funciona", "How it works")).strong());
        ui.add_space(4.0);
        let steps = [
            t(
                "Importa el plano (PNG, JPG o PDF).",
                "Import the floor plan (PNG, JPG or PDF).",
            ),
            t(
                "Calibra la escala con una distancia conocida.",
                "Calibrate the scale with a known distance.",
            ),
            t(
                "Dibuja las paredes eligiendo su material.",
                "Draw the walls, choosing their material.",
            ),
            t(
                "Coloca los puntos de acceso y revisa la cobertura.",
                "Place access points and check the coverage.",
            ),
            t(
                "Opcional: mide en sitio y compara.",
                "Optional: survey on site and compare.",
            ),
            t("Exporta el reporte PDF.", "Export the PDF report."),
        ];
        egui::Frame::group(ui.style())
            .inner_margin(12.0)
            .show(ui, |ui| {
                ui.set_width(420.0);
                for (i, s) in steps.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(format!("{}", i + 1))
                                .strong()
                                .color(style::ACCENT),
                        );
                        ui.label(*s);
                    });
                }
            });
    });
}

fn shortcuts_window(ctx: &egui::Context, open: &mut bool) {
    egui::Window::new(t("Atajos de teclado", "Keyboard shortcuts"))
        .open(open)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            egui::Grid::new("shortcuts").striped(true).show(ui, |ui| {
                let rows = [
                    (
                        "Ctrl+N / Ctrl+O",
                        t("Nuevo desde plano / Abrir", "New from plan / Open"),
                    ),
                    (
                        "Ctrl+S / Ctrl+Shift+S",
                        t("Guardar / Guardar como", "Save / Save as"),
                    ),
                    ("Ctrl+Z / Ctrl+Y", t("Deshacer / Rehacer", "Undo / Redo")),
                    (
                        "V  W  A  C  S",
                        t(
                            "Seleccionar, Paredes, APs, Escala, Medir",
                            "Select, Walls, APs, Scale, Measure",
                        ),
                    ),
                    (
                        "1 – 6",
                        t(
                            "Señal, SNR, SINR, Nº de APs, Solapamiento, Cumple requisitos",
                            "Signal, SNR, SINR, AP count, Overlap, Meets requirements",
                        ),
                    ),
                    (
                        "H",
                        t(
                            "Mostrar u ocultar el mapa de calor",
                            "Show or hide the heat map",
                        ),
                    ),
                    (
                        "F",
                        t(
                            "Ajustar el plano a la ventana",
                            "Fit the plan to the window",
                        ),
                    ),
                    ("Supr", t("Eliminar la selección", "Delete the selection")),
                    (
                        "Esc",
                        t(
                            "Terminar pared / volver a Seleccionar",
                            "End wall / back to Select",
                        ),
                    ),
                    (
                        t("Rueda / botón central", "Wheel / middle button"),
                        t("Zoom / desplazar", "Zoom / pan"),
                    ),
                ];
                for (k, v) in rows {
                    ui.strong(k);
                    ui.label(v);
                    ui.end_row();
                }
            });
        });
}

/// Name of a heat map metric in the UI language.
pub fn metric_label(m: MetricKind) -> &'static str {
    match m {
        MetricKind::Rssi => t("Intensidad de señal", "Signal strength"),
        MetricKind::Snr => t("Señal/ruido (SNR)", "Signal-to-noise (SNR)"),
        MetricKind::Sinr => t(
            "Señal/interferencia (SINR)",
            "Signal-to-interference (SINR)",
        ),
        MetricKind::ApCount => t("Nº de APs que se escuchan", "Number of APs heard"),
        MetricKind::ChannelOverlap => t("Solapamiento de canales", "Channel overlap"),
        MetricKind::Coverage => t("Cumple requisitos", "Meets requirements"),
    }
}

/// One-sentence explanation of a metric.
pub fn metric_help(m: MetricKind) -> &'static str {
    match m {
        MetricKind::Rssi => t(
            "Potencia del AP más fuerte en cada punto, en dBm. Más cerca de 0 es mejor; −67 dBm es lo típico para voz y video.",
            "Power of the strongest AP at each point, in dBm. Closer to 0 is better; −67 dBm is typical for voice and video.",
        ),
        MetricKind::Snr => t(
            "Cuánto supera la señal al ruido de fondo, en dB. 25 dB o más es una buena conexión.",
            "How far the signal is above the noise floor, in dB. 25 dB or more is a good link.",
        ),
        MetricKind::Sinr => t(
            "Como la SNR, pero restando también la interferencia de otros APs en canales solapados.",
            "Like SNR, but also counting interference from other APs on overlapping channels.",
        ),
        MetricKind::ApCount => t(
            "Cuántos APs se reciben por encima de la señal mínima. 2 o más da respaldo y roaming fluido.",
            "How many APs are received above the minimum signal. 2 or more gives redundancy and smooth roaming.",
        ),
        MetricKind::ChannelOverlap => t(
            "Cuántos otros APs en canales solapados se escuchan. Lo ideal es 0 o 1.",
            "How many other APs on overlapping channels are heard. Ideally 0 or 1.",
        ),
        MetricKind::Coverage => t(
            "Verde donde se cumplen la señal y la SNR mínimas configuradas; rojo donde no.",
            "Green where the configured minimum signal and SNR are met; red where not.",
        ),
    }
}

pub fn source_label(s: Source) -> &'static str {
    match s {
        Source::Predicted => t("Predicción", "Prediction"),
        Source::Measured => t("Medición", "Measurement"),
        Source::Difference => t("Diferencia", "Difference"),
    }
}

fn source_help(s: Source) -> &'static str {
    match s {
        Source::Predicted => t(
            "Calculado a partir de los APs, paredes y materiales del diseño.",
            "Computed from the design's APs, walls and materials.",
        ),
        Source::Measured => t(
            "Interpolado a partir de los escaneos WiFi hechos en sitio (pestaña Mediciones).",
            "Interpolated from the on-site WiFi scans (Survey tab).",
        ),
        Source::Difference => t(
            "Medido − predicho. Rojo: la señal real es más débil que la predicha. Azul: más fuerte.",
            "Measured − predicted. Red: the real signal is weaker than predicted. Blue: stronger.",
        ),
    }
}
