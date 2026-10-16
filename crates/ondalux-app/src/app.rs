//! Application state, menus, shortcuts and layout.

use crate::canvas::CanvasState;
use crate::catalog_ui::{self, CatalogState};
use crate::document::Document;
use crate::export_ui::{self, ExportState};
use crate::heatmap::{self, Comparison, Field, Source, Worker};
use crate::i18n::{self, t, ThemeChoice, UiPrefs};
use crate::panels;
use crate::style::{self, icon};
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
use ondalux_io::{decode_plan, ProjectFile};
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
            Tool::Select => icon::CURSOR,
            Tool::Wall => icon::WALL,
            Tool::Ap => icon::BROADCAST,
            Tool::Calibrate => icon::RULER,
            Tool::Survey => icon::CELL_SIGNAL_HIGH,
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
    /// Thickness of new walls; `None` = the material's typical one.
    pub wall_thickness_cm: Option<f32>,
    pub heat: HeatmapView,
    worker: Worker,
    pub survey: SurveyState,
    pub right_tab: RightTab,
    pub catalog: CatalogState,
    pub export: ExportState,
    pub walls: crate::walls_ui::WallDetectState,
    pub view3d: crate::view3d::View3dState,
    pub floors: crate::floors_ui::FloorState,
    pub update: crate::update::UpdateState,
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
    /// Logo texture, created on first use.
    logo: Option<egui::TextureHandle>,
}

/// Source code repository.
pub const REPO_URL: &str = "https://github.com/auchavez98/ondalux-heatmap";

/// How long status bar messages stay visible.
const STATUS_TTL: Duration = Duration::from_secs(6);

impl OndaluxApp {
    /// `path` (from the command line) may be a project or a floor plan image.
    pub fn new(cc: &eframe::CreationContext<'_>, path: Option<PathBuf>) -> Self {
        let prefs = UiPrefs::load();
        i18n::set_english(prefs.english);
        style::install_fonts(&cc.egui_ctx);
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
            wall_thickness_cm: None,
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
            walls: Default::default(),
            view3d: Default::default(),
            floors: Default::default(),
            update: Default::default(),
            show_materials: false,
            show_settings: false,
            show_about: false,
            pending: None,
            allow_quit: false,
            status: None,
            status_at: None,
            prefs,
            show_shortcuts: false,
            logo: None,
        };
        if let Some(e) = catalog_error {
            app.error(e);
        }
        crate::update::check_on_startup(&mut app, &cc.egui_ctx);
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

    /// The logo as a texture (rendered once at a size sharp on HiDPI screens).
    fn logo_texture(&mut self, ctx: &egui::Context) -> egui::TextureHandle {
        self.logo
            .get_or_insert_with(|| {
                let size = 192;
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [size as usize, size as usize],
                    &ondalux_report::logo::rgba(size),
                );
                ctx.load_texture("ondalux-logo", image, egui::TextureOptions::LINEAR)
            })
            .clone()
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

    /// Loads the active floor's plan into a texture.
    pub fn load_plan_texture(&mut self, ctx: &egui::Context) {
        self.plan_texture = self.doc.as_ref().and_then(|d| {
            let plan = d.plan_image()?;
            match decode_plan(plan) {
                Ok(img) => {
                    // Only the on-screen copy is reduced if the GPU cannot
                    // hold it; the plan keeps its full resolution.
                    let max_side = ctx.input(|i| i.max_texture_side);
                    let (w, h, rgba) =
                        fit_texture(img.width as usize, img.height as usize, img.rgba, max_side);
                    let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
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
    }

    fn set_document(&mut self, ctx: &egui::Context, doc: Option<Document>) {
        self.doc = doc;
        self.floors = Default::default();
        crate::floors_ui::sync(self, ctx);
        self.selection = Selection::None;
        self.walls = Default::default();
        self.view3d = Default::default();
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

    /// New project from a plan file. A PDF with several pages becomes a
    /// building with one floor per page (first page = lowest floor).
    fn import_plan_path(&mut self, ctx: &egui::Context, path: PathBuf) {
        match ondalux_io::import_plan_pages(&path) {
            Ok(pages) if !pages.is_empty() => {
                let name = path
                    .file_stem()
                    .map_or_else(|| "Untitled".into(), |s| s.to_string_lossy().into_owned());
                let n = pages.len();
                let mut pages = pages.into_iter();
                let first = pages.next().expect("at least one page");
                let project = Project::new(name, first.decoded.width, first.decoded.height);
                let mut doc = Document::new(project, None, None);
                doc.set_assets(crate::floors_ui::assets_of(&first));
                if n > 1 {
                    // Floors stay unnamed: they show as "Floor n" in the UI language.
                    for (i, page) in pages.enumerate() {
                        let floor = ondalux_core::project::Floor::new(
                            Default::default(),
                            page.decoded.width,
                            page.decoded.height,
                        );
                        let assets = crate::floors_ui::assets_of(&page);
                        doc.project.insert_floor(i + 1, floor);
                        doc.floors.insert(i + 1, std::sync::Arc::new(assets));
                    }
                }
                doc.dirty = true;
                self.set_document(ctx, Some(doc));
                // Beta, opt-in: propose walls right away for review.
                if self.prefs.auto_detect_walls {
                    crate::walls_ui::start(self, ctx);
                }
                self.tool = Tool::Calibrate;
                if n > 1 {
                    self.info(tr!(
                        "PDF de {n} páginas: se creó un piso por página. Calibra la escala del piso 1; se aplicará a los pisos del mismo tamaño. Si alguna página no es un plano, elimina ese piso.",
                        "{n}-page PDF: one floor was created per page. Calibrate the scale of floor 1; it will be applied to floors of the same size. If a page is not a floor plan, delete that floor."
                    ));
                } else {
                    self.info(t("Plano importado. Calibra la escala: haz clic en dos puntos de distancia conocida.", "Floor plan imported. Calibrate the scale: click two points a known distance apart."));
                }
            }
            Ok(_) => self.error(tr!(
                "{} no tiene páginas.",
                "{} has no pages.",
                path.display()
            )),
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
                self.prefs.remember(&path);
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
                self.prefs.remember(&path);
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
        if key(Key::D) {
            self.view3d.enabled = !self.view3d.enabled;
        }
        if let Some(doc) = &self.doc {
            let i = doc.project.floor_index;
            if key(Key::PageUp) {
                crate::floors_ui::switch(self, i + 1);
            } else if key(Key::PageDown) && i > 0 {
                crate::floors_ui::switch(self, i - 1);
            }
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

    /// Top bar: logo and project name, menus, and quick actions.
    fn header(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let p = style::palette(ui);
        let logo = self.logo_texture(&ctx);
        ui.horizontal(|ui| {
            ui.set_min_height(36.0);
            ui.add(egui::Image::new(&logo).fit_to_exact_size(egui::vec2(24.0, 24.0)));
            ui.label(style::bold("Ondalux").size(16.0));
            if let Some(doc) = &self.doc {
                ui.label(egui::RichText::new("/").color(p.border).size(16.0));
                ui.label(egui::RichText::new(doc.title()).color(p.muted));
            }
            ui.add_space(10.0);
            self.menu_bar(ui);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.doc.is_some() {
                    let dirty = self.is_dirty();
                    let text = format!("{}  {}", icon::FLOPPY_DISK, t("Guardar", "Save"));
                    let button = if dirty {
                        egui::Button::new(style::bold(text).color(egui::Color32::WHITE))
                            .fill(p.accent)
                            .stroke(egui::Stroke::NONE)
                    } else {
                        egui::Button::new(text)
                    };
                    if ui
                        .add(button.corner_radius(8))
                        .on_hover_text(if dirty {
                            t(
                                "Hay cambios sin guardar (Ctrl+S)",
                                "Unsaved changes (Ctrl+S)",
                            )
                        } else {
                            t("Todo guardado (Ctrl+S)", "Everything saved (Ctrl+S)")
                        })
                        .clicked()
                    {
                        self.save(false);
                    }
                    let (can_undo, can_redo) = self
                        .doc
                        .as_ref()
                        .map_or((false, false), |d| (d.can_undo(), d.can_redo()));
                    ui.add_enabled_ui(can_redo, |ui| {
                        if style::icon_button(
                            ui,
                            icon::ARROW_U_UP_RIGHT,
                            t("Rehacer (Ctrl+Y)", "Redo (Ctrl+Y)"),
                        )
                        .clicked()
                        {
                            self.redo();
                        }
                    });
                    ui.add_enabled_ui(can_undo, |ui| {
                        if style::icon_button(
                            ui,
                            icon::ARROW_U_UP_LEFT,
                            t("Deshacer (Ctrl+Z)", "Undo (Ctrl+Z)"),
                        )
                        .clicked()
                        {
                            self.undo();
                        }
                    });
                    ui.add_space(4.0);
                }
                let dark = ui.visuals().dark_mode;
                let (glyph, tip) = if dark {
                    (icon::SUN, t("Tema claro", "Light theme"))
                } else {
                    (icon::MOON, t("Tema oscuro", "Dark theme"))
                };
                if style::icon_button(ui, glyph, tip).clicked() {
                    self.prefs.theme = if dark {
                        ThemeChoice::Light
                    } else {
                        ThemeChoice::Dark
                    };
                    style::apply(&ctx, self.prefs.theme);
                    self.prefs.save();
                }
            });
        });
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let short = |m: Modifiers, k: Key| ctx.format_shortcut(&KeyboardShortcut::new(m, k));
        let has_doc = self.doc.is_some();
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button(t("Archivo", "File"), |ui| {
                if ui
                    .add(
                        egui::Button::new(format!(
                            "{}  {}",
                            icon::FILE_PLUS,
                            t("Nuevo desde plano…", "New from floor plan…")
                        ))
                        .shortcut_text(short(Modifiers::COMMAND, Key::N)),
                    )
                    .clicked()
                {
                    self.guarded(&ctx, Pending::NewFromPlan);
                }
                if ui
                    .add(
                        egui::Button::new(format!(
                            "{}  {}",
                            icon::FOLDER_OPEN,
                            t("Abrir…", "Open…")
                        ))
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
                        egui::Button::new(format!(
                            "{}  {}",
                            icon::FLOPPY_DISK,
                            t("Guardar", "Save")
                        ))
                        .shortcut_text(short(Modifiers::COMMAND, Key::S)),
                    )
                    .clicked()
                {
                    self.save(false);
                }
                if ui
                    .add_enabled(
                        has_doc,
                        egui::Button::new(format!(
                            "{}  {}",
                            icon::COPY,
                            t("Guardar como…", "Save as…")
                        ))
                        .shortcut_text(short(Modifiers::COMMAND | Modifiers::SHIFT, Key::S)),
                    )
                    .clicked()
                {
                    self.save(true);
                }
                ui.separator();
                ui.add_enabled_ui(has_doc, |ui| {
                    ui.menu_button(
                        format!("{}  {}", icon::EXPORT, t("Exportar", "Export")),
                        |ui| {
                            if ui
                                .button(format!(
                                    "{}  {}",
                                    icon::FILE_PDF,
                                    t("Reporte PDF…", "PDF report…")
                                ))
                                .clicked()
                            {
                                self.export.report_open = true;
                            }
                            if ui
                                .button(format!(
                                    "{}  {}",
                                    icon::IMAGE,
                                    t("Imagen del mapa (PNG)…", "Heat map image (PNG)…")
                                ))
                                .clicked()
                            {
                                export_ui::export_png(self);
                            }
                            if ui
                                .button(format!(
                                    "{}  {}",
                                    icon::FILE_CSV,
                                    t("Lista de APs (CSV)…", "Access point list (CSV)…")
                                ))
                                .clicked()
                            {
                                export_ui::export_ap_csv(self);
                            }
                            if ui
                                .button(format!(
                                    "{}  {}",
                                    icon::FILE_CSV,
                                    t("Mediciones (CSV)…", "Survey measurements (CSV)…")
                                ))
                                .clicked()
                            {
                                survey_ui::export_csv(self);
                            }
                        },
                    );
                });
                ui.separator();
                if ui
                    .add_enabled(
                        has_doc,
                        egui::Button::new(format!(
                            "{}  {}",
                            icon::X,
                            t("Cerrar proyecto", "Close project")
                        )),
                    )
                    .clicked()
                {
                    self.guarded(&ctx, Pending::Close);
                }
                if ui
                    .button(format!("{}  {}", icon::SIGN_OUT, t("Salir", "Quit")))
                    .clicked()
                {
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
                        egui::Button::new(format!(
                            "{}  {}",
                            icon::ARROW_U_UP_LEFT,
                            t("Deshacer", "Undo")
                        ))
                        .shortcut_text(short(Modifiers::COMMAND, Key::Z)),
                    )
                    .clicked()
                {
                    self.undo();
                }
                if ui
                    .add_enabled(
                        can_redo,
                        egui::Button::new(format!(
                            "{}  {}",
                            icon::ARROW_U_UP_RIGHT,
                            t("Rehacer", "Redo")
                        ))
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
                        egui::Button::new(format!(
                            "{}  {}",
                            icon::TRASH,
                            t("Eliminar selección", "Delete selection")
                        ))
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
                        egui::Button::new(format!(
                            "{}  {}",
                            icon::WALL,
                            t("Materiales de pared…", "Wall materials…")
                        )),
                    )
                    .clicked()
                {
                    self.show_materials = true;
                }
                if ui
                    .add_enabled(
                        has_doc,
                        egui::Button::new(format!(
                            "{}  {}",
                            icon::MAGIC_WAND,
                            t(
                                "Detectar muros del plano (beta)…",
                                "Detect walls from the plan (beta)…"
                            )
                        )),
                    )
                    .clicked()
                {
                    crate::walls_ui::start(self, &ctx);
                }
                if ui
                    .button(format!(
                        "{}  {}",
                        icon::BOOK_OPEN,
                        t("Catálogo de APs…", "AP catalog…")
                    ))
                    .clicked()
                {
                    self.catalog.open = true;
                }
                if ui
                    .add_enabled(
                        has_doc,
                        egui::Button::new(format!(
                            "{}  {}",
                            icon::GEAR,
                            t(
                                "Requisitos y modelo de predicción…",
                                "Requirements and prediction model…",
                            )
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
                        egui::Button::new(format!(
                            "{}  {}",
                            icon::CORNERS_OUT,
                            t("Ajustar plano a la ventana", "Fit plan to window")
                        ))
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
                ui.menu_button(
                    format!("{}  {}", icon::TRANSLATE, t("Idioma", "Language")),
                    |ui| {
                        ui.radio_value(&mut prefs.english, false, "Español");
                        ui.radio_value(&mut prefs.english, true, "English");
                    },
                );
                ui.menu_button(format!("{}  {}", icon::PALETTE, t("Tema", "Theme")), |ui| {
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
                    .button(format!(
                        "{}  {}",
                        icon::KEYBOARD,
                        t("Atajos de teclado", "Keyboard shortcuts")
                    ))
                    .clicked()
                {
                    self.show_shortcuts = true;
                }
                ui.separator();
                let site = if i18n::is_english() {
                    "https://ondaluxheatmap.com/"
                } else {
                    "https://ondaluxheatmap.com/es/"
                };
                if ui
                    .button(format!(
                        "{}  {}",
                        icon::GLOBE,
                        t("Sitio web (guía y descargas)", "Website (guide and downloads)")
                    ))
                    .on_hover_text(site)
                    .clicked()
                {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(site));
                }
                if ui
                    .button(format!(
                        "{}  {}",
                        icon::GITHUB_LOGO,
                        t("Código fuente en GitHub", "Source code on GitHub")
                    ))
                    .on_hover_text(REPO_URL)
                    .clicked()
                {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(REPO_URL));
                }
                if ui
                    .button(format!(
                        "{}  {}",
                        icon::BUG,
                        t("Reportar un problema", "Report a problem")
                    ))
                    .on_hover_text(format!("{REPO_URL}/issues"))
                    .clicked()
                {
                    ui.ctx()
                        .open_url(egui::OpenUrl::new_tab(format!("{REPO_URL}/issues")));
                }
                ui.separator();
                let checking = self.update.busy();
                if ui
                    .add_enabled(
                        !checking,
                        egui::Button::new(format!(
                            "{}  {}",
                            icon::ARROW_CIRCLE_UP,
                            if checking {
                                t("Buscando actualizaciones…", "Checking for updates…")
                            } else {
                                t("Buscar actualizaciones", "Check for updates")
                            }
                        )),
                    )
                    .clicked()
                {
                    let ctx = ui.ctx().clone();
                    crate::update::check(self, &ctx, true);
                }
                let mut auto = self.prefs.check_updates;
                if ui
                    .checkbox(
                        &mut auto,
                        t(
                            "Avisarme de nuevas versiones al iniciar",
                            "Tell me about new versions at startup",
                        ),
                    )
                    .on_hover_text(t(
                        "Una vez al día consulta ondaluxheatmap.com/version.json. No envía datos de tus proyectos.",
                        "Once a day it reads ondaluxheatmap.com/version.json. No project data is sent.",
                    ))
                    .changed()
                {
                    self.prefs.check_updates = auto;
                    self.prefs.save();
                }
                ui.separator();
                if ui
                    .button(format!(
                        "{}  {}",
                        icon::INFO,
                        t("Acerca de Ondalux Heatmap", "About Ondalux Heatmap")
                    ))
                    .clicked()
                {
                    self.show_about = true;
                }
            });
        });
    }

    /// Left column: tools, then the options of the active tool.
    fn tool_panel(&mut self, ui: &mut egui::Ui) {
        let p = style::palette(ui);
        style::section(ui, t("Herramientas", "Tools"));
        ui.spacing_mut().item_spacing.y = 4.0;
        for tool in Tool::ALL {
            let r =
                style::tool_button(ui, tool.icon(), tool.label(), tool.key(), self.tool == tool)
                    .on_hover_text(tool.hint());
            if r.clicked() {
                self.set_tool(tool);
            }
        }
        ui.spacing_mut().item_spacing.y = 8.0;
        ui.add_space(10.0);
        let mut detect = false;
        let Some(doc) = self.doc.as_ref() else {
            return;
        };
        style::card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(self.tool.icon())
                        .size(16.0)
                        .color(p.accent),
                );
                ui.label(style::bold(self.tool.label()));
            });
            match self.tool {
                Tool::Select => {
                    ui.label(
                        egui::RichText::new(t(
                            "Haz clic en un AP, pared o punto medido para ver y editar sus datos. Arrastra para moverlo.",
                            "Click an AP, wall or survey point to see and edit it. Drag to move it.",
                        ))
                        .color(p.muted),
                    );
                }
                Tool::Wall => {
                    ui.label(egui::RichText::new(t("Material", "Material")).color(p.muted));
                    let w = ui.available_width();
                    crate::panels::material_combo(
                        ui,
                        "wall-material",
                        &doc.project.materials,
                        &mut self.wall_material,
                        w,
                    );
                    ui.label(egui::RichText::new(t("Grosor", "Thickness")).color(p.muted));
                    let typical = doc
                        .project
                        .materials
                        .get(self.wall_material)
                        .and_then(|m| m.thickness_cm)
                        .unwrap_or(ondalux_core::material::DEFAULT_WALL_THICKNESS_CM);
                    crate::panels::thickness_edit(ui, &mut self.wall_thickness_cm, typical);
                    ui.label(
                        egui::RichText::new(t(
                            "Clic para agregar puntos. Mayús = ángulos de 45°. Esc o doble clic termina la pared.",
                            "Click to add points. Shift = 45° angles. Esc or double-click ends the wall.",
                        ))
                        .color(p.muted),
                    );
                    if ui
                        .button(format!(
                            "{}  {}",
                            icon::SLIDERS_HORIZONTAL,
                            t("Editar materiales…", "Edit materials…")
                        ))
                        .clicked()
                    {
                        self.show_materials = true;
                    }
                    detect = ui
                        .button(format!(
                            "{}  {}",
                            icon::MAGIC_WAND,
                            t("Detectar muros (beta)…", "Detect walls (beta)…")
                        ))
                        .clicked();
                }
                Tool::Ap => {
                    ui.label(
                        egui::RichText::new(t("Modelo para nuevos APs", "Model for new APs"))
                            .color(p.muted),
                    );
                    catalog_ui::model_combo(
                        ui,
                        "new-ap-model",
                        &self.catalog.catalog,
                        &mut self.catalog.new_ap_model,
                        t("Genérico", "Generic"),
                    );
                    ui.label(
                        egui::RichText::new(t(
                            "Clic en el plano para colocar un AP. Sus radios se llenan con los datos del modelo.",
                            "Click on the plan to place an AP. Its radios are filled from the model.",
                        ))
                        .color(p.muted),
                    );
                    if ui
                        .button(format!(
                            "{}  {}",
                            icon::BOOK_OPEN,
                            t("Ver catálogo…", "Browse catalog…")
                        ))
                        .clicked()
                    {
                        self.catalog.open = true;
                    }
                }
                Tool::Calibrate => {
                    match doc.project.plan.calibration {
                        Some(c) => {
                            style::badge(
                                ui,
                                &format!(
                                    "{} · {:.1} px/m",
                                    t("Calibrada", "Calibrated"),
                                    c.px_per_m()
                                ),
                                p.success,
                            );
                        }
                        None => {
                            style::badge(ui, t("Sin calibrar", "Not calibrated"), p.warning);
                        }
                    }
                    ui.label(
                        egui::RichText::new(t(
                            "Haz clic en dos puntos cuya distancia real conozcas (una puerta, una cota) y escribe la distancia.",
                            "Click two points whose real distance you know (a door, a dimension) and type the distance.",
                        ))
                        .color(p.muted),
                    );
                }
                Tool::Survey => {
                    ui.label(
                        egui::RichText::new(t(
                            "Párate en un lugar, haz clic en tu posición en el plano y espera a que termine el escaneo.",
                            "Stand somewhere, click your position on the plan and wait for the scan to finish.",
                        ))
                        .color(p.muted),
                    );
                    if ui
                        .button(format!(
                            "{}  {}",
                            icon::CELL_SIGNAL_HIGH,
                            t("Panel de mediciones", "Survey panel")
                        ))
                        .clicked()
                    {
                        self.right_tab = RightTab::Survey;
                    }
                }
            }
        });

        if detect {
            let ctx = ui.ctx().clone();
            crate::walls_ui::start(self, &ctx);
            return;
        }
        crate::floors_ui::panel(self, ui);
        let Some(doc) = self.doc.as_ref() else {
            return;
        };
        // Project summary.
        let coverage = self.heat.field.as_ref().and_then(|f| f.coverage);
        let p2 = &doc.project;
        ui.add_space(6.0);
        style::section(ui, t("Resumen", "Summary"));
        style::card(ui, |ui| {
            ui.columns(3, |cols| {
                let (cov, color) = match coverage {
                    Some(c) if !p2.aps.is_empty() => (
                        format!("{:.0}%", c * 100.0),
                        if c >= 0.95 {
                            p.success
                        } else if c >= 0.8 {
                            p.warning
                        } else {
                            p.danger
                        },
                    ),
                    _ => ("—".to_owned(), p.muted),
                };
                style::stat(&mut cols[0], &cov, t("cobertura", "coverage"), color);
                style::stat(&mut cols[1], &p2.aps.len().to_string(), "APs", p.text);
                style::stat(
                    &mut cols[2],
                    &p2.walls.len().to_string(),
                    t("paredes", "walls"),
                    p.text,
                );
            });
        });
    }

    /// Controls floating over the floor plan: what the heat map shows,
    /// zoom, cursor readout and notifications.
    fn canvas_overlays(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        let p = style::palette(ui);
        let ctx = ui.ctx().clone();
        // ---- heat map bar (top center) ----
        egui::Area::new(egui::Id::new("heat-bar"))
            .fixed_pos(rect.center_top() + egui::vec2(0.0, 12.0))
            .pivot(egui::Align2::CENTER_TOP)
            .constrain_to(rect)
            .show(&ctx, |ui| {
                style::floating(ui).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let on = self.heat.show;
                        let toggle = egui::Button::new(format!(
                            "{}  {}",
                            if on { icon::EYE } else { icon::EYE_SLASH },
                            t("Mapa de calor", "Heat map")
                        ))
                        .selected(on);
                        if ui.add(toggle).on_hover_text("H").clicked() {
                            self.heat.show = !on;
                        }
                        let in_3d = self.view3d.enabled;
                        if ui
                            .add(egui::Button::new(format!("{}  3D", icon::CUBE)).selected(in_3d))
                            .on_hover_text(t(
                                "Ver el plano en 3D con los muros levantados (D)",
                                "View the plan in 3D with extruded walls (D)",
                            ))
                            .clicked()
                        {
                            self.view3d.enabled = !in_3d;
                        }
                        let floors = self.doc.as_ref().map_or(1, |d| d.project.floor_count());
                        if in_3d && floors > 1 {
                            let others = self.view3d.other_floors;
                            if ui
                                .add(
                                    egui::Button::new(format!(
                                        "{}  {}",
                                        icon::STACK,
                                        t("Pisos", "Floors")
                                    ))
                                    .selected(others),
                                )
                                .on_hover_text(t(
                                    "Mostrar los demás pisos del edificio",
                                    "Show the other floors of the building",
                                ))
                                .clicked()
                            {
                                self.view3d.other_floors = !others;
                            }
                        }
                        ui.add_enabled_ui(on, |ui| {
                            egui::ComboBox::from_id_salt("heatmap-metric")
                                .selected_text(metric_label(self.heat.metric))
                                .width(175.0)
                                .show_ui(ui, |ui| {
                                    for (i, m) in MetricKind::ALL.into_iter().enumerate() {
                                        ui.selectable_value(
                                            &mut self.heat.metric,
                                            m,
                                            metric_label(m),
                                        )
                                        .on_hover_text(
                                            format!(
                                                "{}\n\n{}: {}",
                                                metric_help(m),
                                                t("Atajo", "Shortcut"),
                                                i + 1
                                            ),
                                        );
                                    }
                                })
                                .response
                                .on_hover_text(metric_help(self.heat.metric));
                            // Band as a segmented control.
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 2.0;
                                for b in Band::ALL {
                                    ui.selectable_value(&mut self.heat.band, b, b.label());
                                }
                            });
                            egui::ComboBox::from_id_salt("heatmap-source")
                                .selected_text(format!(
                                    "{}  {}",
                                    source_icon(self.heat.source),
                                    source_label(self.heat.source)
                                ))
                                .width(132.0)
                                .show_ui(ui, |ui| {
                                    for s in Source::ALL {
                                        ui.selectable_value(
                                            &mut self.heat.source,
                                            s,
                                            format!("{}  {}", source_icon(s), source_label(s)),
                                        )
                                        .on_hover_text(source_help(s));
                                    }
                                })
                                .response
                                .on_hover_text(source_help(self.heat.source));
                            // Opacity only when there is room for it.
                            if rect.width() > 820.0 {
                                ui.label(egui::RichText::new(icon::CIRCLE_HALF).color(p.muted))
                                    .on_hover_text(t("Opacidad", "Opacity"));
                                ui.spacing_mut().slider_width = 64.0;
                                ui.add(
                                    egui::Slider::new(&mut self.heat.opacity, 0.1..=1.0)
                                        .show_value(false),
                                )
                                .on_hover_text(t("Opacidad", "Opacity"));
                            }
                        });
                        if self.worker.busy() {
                            ui.spinner();
                        }
                    });
                });
            });

        // ---- empty state ----
        if let Some(doc) = &self.doc {
            if self.heat.show && doc.project.aps.is_empty() {
                egui::Area::new(egui::Id::new("empty-hint"))
                    .fixed_pos(rect.center_top() + egui::vec2(0.0, 72.0))
                    .pivot(egui::Align2::CENTER_TOP)
                    .interactable(false)
                    .show(&ctx, |ui| {
                        style::floating(ui).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(icon::LIGHTBULB).color(p.warning).size(16.0));
                                ui.label(t(
                                    "Coloca puntos de acceso con la herramienta AP (A) para ver el mapa de calor",
                                    "Place access points with the AP tool (A) to see the heat map",
                                ));
                            });
                        });
                    });
            }
        }

        // ---- zoom (bottom right) ----
        egui::Area::new(egui::Id::new("zoom-bar"))
            .fixed_pos(rect.right_bottom() - egui::vec2(12.0, 12.0))
            .pivot(egui::Align2::RIGHT_BOTTOM)
            .show(&ctx, |ui| {
                style::floating(ui).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        let center = rect.center();
                        if style::icon_button(
                            ui,
                            icon::MAGNIFYING_GLASS_MINUS,
                            t("Alejar", "Zoom out"),
                        )
                        .clicked()
                        {
                            if self.view3d.enabled {
                                self.view3d.zoom(1.25);
                            } else {
                                self.view.zoom_around(center, 1.0 / 1.25);
                                self.canvas.stop_auto_fit();
                            }
                        }
                        let fit_zoom = self
                            .doc
                            .as_ref()
                            .map(|d| {
                                let plan = &d.project.plan;
                                View::fit(
                                    rect,
                                    egui::vec2(plan.width_px as f32, plan.height_px as f32),
                                )
                                .zoom
                            })
                            .unwrap_or(1.0);
                        let pct = (self.view.zoom / fit_zoom * 100.0).round();
                        let label = if self.view3d.enabled {
                            "3D".to_owned()
                        } else {
                            format!("{pct:.0} %")
                        };
                        ui.add_sized(
                            [52.0, 30.0],
                            egui::Label::new(egui::RichText::new(label).color(p.muted)),
                        );
                        if style::icon_button(
                            ui,
                            icon::MAGNIFYING_GLASS_PLUS,
                            t("Acercar", "Zoom in"),
                        )
                        .clicked()
                        {
                            if self.view3d.enabled {
                                self.view3d.zoom(1.0 / 1.25);
                            } else {
                                self.view.zoom_around(center, 1.25);
                                self.canvas.stop_auto_fit();
                            }
                        }
                        if style::icon_button(
                            ui,
                            icon::CORNERS_OUT,
                            t("Ajustar a la ventana (F)", "Fit to window (F)"),
                        )
                        .clicked()
                        {
                            if self.view3d.enabled {
                                self.view3d.reset();
                            }
                            self.fit_pending = true;
                        }
                    });
                });
            });

        // ---- cursor readout / tool hint (bottom center) ----
        let readout = self.canvas.hover_text.take();
        // A notification takes this spot while it is shown.
        if self.status.is_some() {
            return;
        }
        // Above the legend and zoom row, so nothing overlaps.
        egui::Area::new(egui::Id::new("readout"))
            .fixed_pos(rect.center_bottom() - egui::vec2(0.0, 92.0))
            .pivot(egui::Align2::CENTER_BOTTOM)
            .interactable(false)
            .show(&ctx, |ui| {
                style::floating(ui).show(ui, |ui| {
                    ui.set_max_width((rect.width() - 48.0).min(620.0));
                    ui.horizontal(|ui| match &readout {
                        Some(text) => {
                            ui.label(egui::RichText::new(icon::CROSSHAIR).color(p.muted));
                            ui.label(text);
                        }
                        None => {
                            ui.label(egui::RichText::new(self.tool.icon()).color(p.accent));
                            ui.label(egui::RichText::new(self.tool.hint()).color(p.muted));
                        }
                    });
                });
            });
    }

    /// Notification in the bottom-right corner of the window, fading out.
    fn toast(&mut self, ctx: &egui::Context, area: egui::Rect) {
        let (Some((msg, is_error)), Some(at)) = (self.status.clone(), self.status_at) else {
            return;
        };
        let age = at.elapsed();
        if age > STATUS_TTL {
            self.status = None;
            self.status_at = None;
            return;
        }
        let fade =
            1.0 - ((age.as_secs_f32() - (STATUS_TTL.as_secs_f32() - 0.6)) / 0.6).clamp(0.0, 1.0);
        ctx.request_repaint_after(Duration::from_millis(50));
        egui::Area::new(egui::Id::new("toast"))
            .order(egui::Order::Foreground)
            .fixed_pos(area.center_bottom() - egui::vec2(0.0, 92.0))
            .pivot(egui::Align2::CENTER_BOTTOM)
            .show(ctx, |ui| {
                ui.set_opacity(fade);
                let p = style::palette(ui);
                let (glyph, color) = if is_error {
                    (icon::WARNING_CIRCLE, p.danger)
                } else {
                    (icon::CHECK_CIRCLE, p.success)
                };
                style::floating(ui)
                    .inner_margin(egui::Margin::symmetric(14, 10))
                    .show(ui, |ui| {
                        ui.set_max_width(420.0);
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(glyph).size(18.0).color(color));
                            ui.add(egui::Label::new(msg).wrap());
                            if style::icon_button(ui, icon::X, t("Cerrar", "Dismiss")).clicked() {
                                self.status = None;
                                self.status_at = None;
                            }
                        });
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
            aps: p.radiating_aps(),
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
        crate::walls_ui::window(self, ctx);
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
        crate::floors_ui::sync(self, &ctx);
        crate::update::poll(self);
        self.poll_heatmap(&ctx);
        survey_ui::poll(self);
        export_ui::poll(self);
        crate::walls_ui::poll(self, &ctx);

        let p = style::palette(ui);
        let side = |fill| {
            egui::Frame::new()
                .fill(fill)
                .inner_margin(egui::Margin::symmetric(14, 12))
                .stroke(egui::Stroke::new(1.0, p.border))
        };
        egui::Panel::top("header")
            .frame(
                egui::Frame::new()
                    .fill(p.panel)
                    .inner_margin(egui::Margin::symmetric(14, 6))
                    .stroke(egui::Stroke::new(1.0, p.border)),
            )
            .show(ui, |ui| self.header(ui));
        if self.doc.is_some() {
            egui::Panel::left("tools")
                .resizable(false)
                .exact_size(236.0)
                .frame(side(p.panel))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.tool_panel(ui));
                });
            egui::Panel::right("inspector")
                .resizable(true)
                .default_size(340.0)
                .min_size(300.0)
                .frame(side(p.panel))
                .show(ui, |ui| {
                    // Segmented tabs.
                    egui::Frame::new()
                        .fill(p.surface)
                        .corner_radius(10)
                        .inner_margin(egui::Margin::same(3))
                        .show(ui, |ui| {
                            ui.columns(2, |cols| {
                                for (col, tab, glyph, label) in [
                                    (
                                        0,
                                        RightTab::Inspector,
                                        icon::SLIDERS_HORIZONTAL,
                                        t("Propiedades", "Properties"),
                                    ),
                                    (
                                        1,
                                        RightTab::Survey,
                                        icon::CELL_SIGNAL_HIGH,
                                        t("Mediciones", "Survey"),
                                    ),
                                ] {
                                    let ui = &mut cols[col];
                                    let on = self.right_tab == tab;
                                    let text = format!("{glyph}  {label}");
                                    let button = if on {
                                        egui::Button::new(style::bold(text).color(p.text))
                                            .fill(p.panel)
                                            .stroke(egui::Stroke::new(1.0, p.border))
                                    } else {
                                        egui::Button::new(egui::RichText::new(text).color(p.muted))
                                            .frame_when_inactive(false)
                                    };
                                    if ui
                                        .add_sized(
                                            [ui.available_width(), 30.0],
                                            button.corner_radius(8),
                                        )
                                        .clicked()
                                    {
                                        self.right_tab = tab;
                                    }
                                }
                            });
                        });
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical().show(ui, |ui| match self.right_tab {
                        RightTab::Inspector => panels::inspector(self, ui),
                        RightTab::Survey => survey_ui::panel(self, ui),
                    });
                });
        }
        let mut toast_area = ctx.content_rect();
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(p.backdrop))
            .show(ui, |ui| {
                let rect = ui.max_rect();
                toast_area = rect;
                if self.doc.is_some() {
                    if self.view3d.enabled {
                        crate::view3d::show(self, ui);
                    } else {
                        crate::canvas::show(self, ui);
                    }
                    self.canvas_overlays(ui, rect);
                } else {
                    welcome(self, ui);
                }
            });

        self.toast(&ctx, toast_area);
        self.dialogs(&ctx);
        crate::floors_ui::settings_window(self, &ctx);
        crate::update::window(self, &ctx);
        self.request_heatmap();
    }
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// Large clickable card with an icon, a title and one line of text.
fn action_card(ui: &mut egui::Ui, glyph: &str, title: &str, text: &str, primary: bool) -> bool {
    let p = style::palette(ui);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(300.0, 104.0), egui::Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let hovered = response.hovered();
    let painter = ui.painter();
    let (fill, stroke) = match (primary, hovered) {
        (true, _) => (p.accent_soft, egui::Stroke::new(1.5, p.accent)),
        (false, true) => (p.surface_hover, egui::Stroke::new(1.0, p.accent)),
        (false, false) => (p.surface, egui::Stroke::new(1.0, p.border)),
    };
    painter.rect(rect, 14.0, fill, stroke, egui::StrokeKind::Inside);
    let circle = rect.left_center() + egui::vec2(42.0, 0.0);
    painter.circle_filled(circle, 24.0, if primary { p.accent } else { p.accent_soft });
    painter.text(
        circle,
        egui::Align2::CENTER_CENTER,
        glyph,
        egui::FontId::proportional(22.0),
        if primary {
            egui::Color32::WHITE
        } else {
            p.accent
        },
    );
    let x = rect.left() + 82.0;
    painter.text(
        egui::pos2(x, rect.center().y - 10.0),
        egui::Align2::LEFT_CENTER,
        title,
        egui::FontId::new(16.0, style::semibold()),
        p.text,
    );
    painter.text(
        egui::pos2(x, rect.center().y + 13.0),
        egui::Align2::LEFT_CENTER,
        text,
        egui::FontId::proportional(13.0),
        p.muted,
    );
    response.clicked()
}

fn welcome(app: &mut OndaluxApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let p = style::palette(ui);
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.07).max(24.0));
            let logo = app.logo_texture(&ctx);
            ui.add(egui::Image::new(&logo).fit_to_exact_size(egui::vec2(88.0, 88.0)));
            ui.add_space(10.0);
            ui.label(style::bold("Ondalux Heatmap").size(32.0));
            ui.label(
                egui::RichText::new(t(
                    "Diseña y mide redes WiFi con mapas de calor",
                    "Design and survey WiFi networks with heat maps",
                ))
                .size(16.0)
                .color(p.muted),
            );
            ui.add_space(28.0);

            // Two main actions, side by side.
            let width = 300.0 * 2.0 + 16.0;
            ui.allocate_ui(egui::vec2(width, 104.0), |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 16.0;
                    if action_card(
                        ui,
                        icon::FILE_PLUS,
                        t("Nuevo proyecto", "New project"),
                        t(
                            "Empieza importando un plano",
                            "Start by importing a floor plan",
                        ),
                        true,
                    ) {
                        app.guarded(&ctx, Pending::NewFromPlan);
                    }
                    if action_card(
                        ui,
                        icon::FOLDER_OPEN,
                        t("Abrir proyecto", "Open project"),
                        t("Archivo .lux guardado", "A saved .lux file"),
                        false,
                    ) {
                        app.guarded(&ctx, Pending::Open);
                    }
                });
            });
            ui.add_space(10.0);
            ui.allocate_ui(egui::vec2(width, 0.0), |ui| {
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    crate::walls_ui::auto_detect_checkbox(ui, &mut app.prefs);
                });
            });

            // Recent projects.
            let recent: Vec<PathBuf> = app
                .prefs
                .recent
                .iter()
                .filter(|p| p.exists())
                .cloned()
                .collect();
            if !recent.is_empty() {
                ui.add_space(24.0);
                ui.allocate_ui(egui::vec2(width, 0.0), |ui| {
                    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                        style::section(ui, t("Recientes", "Recent"));
                        let mut open = None;
                        style::card(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            for path in &recent {
                                let name = path
                                    .file_stem()
                                    .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
                                let folder = path
                                    .parent()
                                    .map_or_else(String::new, |d| d.display().to_string());
                                let r = recent_row(ui, &name, &folder)
                                    .on_hover_text(path.display().to_string());
                                if r.clicked() {
                                    open = Some(path.clone());
                                }
                            }
                        });
                        if let Some(path) = open {
                            app.open_path(&ctx, path);
                        }
                    });
                });
            }

            // How it works.
            ui.add_space(28.0);
            ui.allocate_ui(egui::vec2(width, 0.0), |ui| {
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    style::section(ui, t("Cómo funciona", "How it works"));
                    let steps = [
                        (
                            icon::IMAGE,
                            t("Importa el plano", "Import the floor plan"),
                            t("PNG, JPG o PDF", "PNG, JPG or PDF"),
                        ),
                        (
                            icon::RULER,
                            t("Calibra la escala", "Calibrate the scale"),
                            t("Con una distancia conocida", "With a known distance"),
                        ),
                        (
                            icon::WALL,
                            t("Dibuja las paredes", "Draw the walls"),
                            t("Eligiendo su material", "Choosing their material"),
                        ),
                        (
                            icon::BROADCAST,
                            t("Coloca los APs", "Place the APs"),
                            t("Y revisa la cobertura", "And check the coverage"),
                        ),
                        (
                            icon::CELL_SIGNAL_HIGH,
                            t("Mide en sitio", "Survey on site"),
                            t("Opcional: compara", "Optional: compare"),
                        ),
                        (
                            icon::FILE_PDF,
                            t("Exporta el reporte", "Export the report"),
                            t("PDF con mapas y hallazgos", "PDF with maps and findings"),
                        ),
                    ];
                    egui::Grid::new("how-it-works")
                        .num_columns(3)
                        .spacing([12.0, 12.0])
                        .show(ui, |ui| {
                            for (i, (glyph, title, text)) in steps.iter().enumerate() {
                                egui::Frame::new()
                                    .fill(p.surface)
                                    .stroke(egui::Stroke::new(1.0, p.border))
                                    .corner_radius(12)
                                    .inner_margin(egui::Margin::same(12))
                                    .show(ui, |ui| {
                                        let w = (width - 24.0) / 3.0 - 24.0;
                                        ui.set_min_width(w);
                                        ui.set_max_width(w);
                                        ui.vertical(|ui| {
                                            ui.spacing_mut().item_spacing.y = 2.0;
                                            ui.horizontal(|ui| {
                                                ui.label(
                                                    egui::RichText::new(*glyph)
                                                        .size(20.0)
                                                        .color(p.accent),
                                                );
                                                ui.label(
                                                    egui::RichText::new(format!("{}", i + 1))
                                                        .size(12.0)
                                                        .color(p.muted),
                                                );
                                            });
                                            ui.label(style::bold(*title));
                                            ui.label(
                                                egui::RichText::new(*text)
                                                    .size(12.5)
                                                    .color(p.muted),
                                            );
                                        });
                                    });
                                if i % 3 == 2 {
                                    ui.end_row();
                                }
                            }
                        });
                });
            });
            ui.add_space(24.0);
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
                    ("D", t("Vista 3D", "3D view")),
                    (
                        t("Re Pág / Av Pág", "Page Up / Page Down"),
                        t("Piso de arriba / de abajo", "Floor above / below"),
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

fn source_icon(s: Source) -> &'static str {
    match s {
        Source::Predicted => icon::CHART_LINE,
        Source::Measured => icon::CELL_SIGNAL_HIGH,
        Source::Difference => icon::GIT_DIFF,
    }
}

/// One recent project: file icon, name and (shortened) folder.
fn recent_row(ui: &mut egui::Ui, name: &str, folder: &str) -> egui::Response {
    let p = style::palette(ui);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 36.0), egui::Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect, 8.0, p.surface_hover);
    }
    painter.text(
        rect.left_center() + egui::vec2(10.0, 0.0),
        egui::Align2::LEFT_CENTER,
        icon::FILE,
        egui::FontId::proportional(17.0),
        p.accent,
    );
    let name_rect = painter.text(
        rect.left_center() + egui::vec2(36.0, 0.0),
        egui::Align2::LEFT_CENTER,
        name,
        egui::FontId::new(14.0, style::semibold()),
        p.text,
    );
    // Keep the end of long folders, which is the informative part.
    let room = ((rect.right() - name_rect.right() - 24.0) / 6.5).max(8.0) as usize;
    let chars: Vec<char> = folder.chars().collect();
    let shown = if chars.len() > room {
        format!(
            "…{}",
            chars[chars.len() - room + 1..].iter().collect::<String>()
        )
    } else {
        folder.to_owned()
    };
    painter.text(
        rect.right_center() - egui::vec2(10.0, 0.0),
        egui::Align2::RIGHT_CENTER,
        shown,
        egui::FontId::proportional(12.0),
        p.muted,
    );
    response
}

/// Shrinks an RGBA image by an integer box filter until both sides fit in
/// `max_side` (the GPU texture limit).
fn fit_texture(w: usize, h: usize, rgba: Vec<u8>, max_side: usize) -> (usize, usize, Vec<u8>) {
    let f = w.max(h).div_ceil(max_side.max(1));
    if f <= 1 {
        return (w, h, rgba);
    }
    let (nw, nh) = (w / f, h / f);
    let mut out = vec![0u8; nw * nh * 4];
    for y in 0..nh {
        for x in 0..nw {
            let mut acc = [0u32; 4];
            for yy in y * f..y * f + f {
                let row = yy * w;
                for xx in x * f..x * f + f {
                    let i = (row + xx) * 4;
                    for c in 0..4 {
                        acc[c] += rgba[i + c] as u32;
                    }
                }
            }
            let n = (f * f) as u32;
            let o = (y * nw + x) * 4;
            for c in 0..4 {
                out[o + c] = (acc[c] / n) as u8;
            }
        }
    }
    (nw, nh, out)
}
