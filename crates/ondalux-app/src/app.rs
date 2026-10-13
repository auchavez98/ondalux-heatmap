//! Application state, menus, shortcuts and layout.

use crate::canvas::CanvasState;
use crate::document::Document;
use crate::heatmap::{self, Worker};
use crate::panels;
use crate::view::View;
use eframe::egui::{self, Key, KeyboardShortcut, Modifiers, TextureHandle, TextureOptions};
use ondalux_core::ap::ApId;
use ondalux_core::channel::Band;
use ondalux_core::grid::GridSpec;
use ondalux_core::material::MaterialId;
use ondalux_core::metrics::{MetricKind, Metrics};
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
}

impl Tool {
    pub const ALL: [Tool; 4] = [Tool::Select, Tool::Wall, Tool::Ap, Tool::Calibrate];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "Select (V)",
            Tool::Wall => "Wall (W)",
            Tool::Ap => "Access point (A)",
            Tool::Calibrate => "Calibrate (C)",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Tool::Select => {
                "Click to select · drag APs or wall ends to move them · drag empty space or middle-drag to pan · wheel to zoom"
            }
            Tool::Wall => {
                "Click to add wall points · Shift snaps to 45° · Esc, right-click or double-click ends the wall"
            }
            Tool::Ap => "Click to place an access point · Esc returns to Select",
            Tool::Calibrate => "Click two points a known distance apart, then enter the distance",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    None,
    Ap(ApId),
    Wall(usize),
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
    pub texture: Option<TextureHandle>,
    pub metrics: Option<Arc<Metrics>>,
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
    pub show_materials: bool,
    pub show_settings: bool,
    show_about: bool,
    pending: Option<Pending>,
    allow_quit: bool,
    /// Message shown in the status bar and whether it is an error.
    pub status: Option<(String, bool)>,
    status_at: Option<Instant>,
}

/// How long status bar messages stay visible.
const STATUS_TTL: Duration = Duration::from_secs(8);

impl OndaluxApp {
    /// `path` (from the command line) may be a project or a floor plan image.
    pub fn new(cc: &eframe::CreationContext<'_>, path: Option<PathBuf>) -> Self {
        cc.egui_ctx.set_theme(egui::Theme::Dark);
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
                texture: None,
                metrics: None,
                last_elapsed: None,
                last_recomputed: 0,
            },
            worker: Worker::spawn(cc.egui_ctx.clone()),
            show_materials: false,
            show_settings: false,
            show_about: false,
            pending: None,
            allow_quit: false,
            status: None,
            status_at: None,
        };
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
                    self.status = Some((format!("Could not decode floor plan: {e}"), true));
                    None
                }
            }
        });
        self.doc = doc;
        self.selection = Selection::None;
        self.canvas = CanvasState::default();
        self.heat.texture = None;
        self.heat.metrics = None;
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
            .set_title("Import floor plan")
            .add_filter("Floor plans", &["png", "jpg", "jpeg", "pdf"])
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
                self.info("Floor plan imported. Calibrate the scale: click two points a known distance apart.");
            }
            Err(e) => self.error(format!("Could not import {}: {e}", path.display())),
        }
    }

    fn open(&mut self, ctx: &egui::Context) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Open project")
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
                self.info(format!("Opened {}", file_name(&path)));
            }
            Err(e) => self.error(format!("Could not open {}: {e}", path.display())),
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
                    .set_title("Save project")
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
                self.info(format!("Saved {}", file_name(&path)));
            }
            Err(e) => self.error(format!("Could not save {}: {e}", path.display())),
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
            _ => return,
        }
        self.selection = Selection::None;
    }

    pub fn set_tool(&mut self, tool: Tool) {
        self.tool = tool;
        self.canvas.cancel();
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
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui
                    .add(egui::Button::new("New from floor plan…").shortcut_text(
                        ctx.format_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::N)),
                    ))
                    .clicked()
                {
                    self.guarded(&ctx, Pending::NewFromPlan);
                }
                if ui
                    .add(egui::Button::new("Open…").shortcut_text(
                        ctx.format_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::O)),
                    ))
                    .clicked()
                {
                    self.guarded(&ctx, Pending::Open);
                }
                ui.separator();
                let has_doc = self.doc.is_some();
                if ui
                    .add_enabled(
                        has_doc,
                        egui::Button::new("Save").shortcut_text(
                            ctx.format_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::S)),
                        ),
                    )
                    .clicked()
                {
                    self.save(false);
                }
                if ui
                    .add_enabled(
                        has_doc,
                        egui::Button::new("Save as…").shortcut_text(ctx.format_shortcut(
                            &KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::S),
                        )),
                    )
                    .clicked()
                {
                    self.save(true);
                }
                ui.separator();
                if ui
                    .add_enabled(has_doc, egui::Button::new("Close project"))
                    .clicked()
                {
                    self.guarded(&ctx, Pending::Close);
                }
                if ui.button("Quit").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| {
                let (can_undo, can_redo) = self
                    .doc
                    .as_ref()
                    .map_or((false, false), |d| (d.can_undo(), d.can_redo()));
                if ui
                    .add_enabled(
                        can_undo,
                        egui::Button::new("Undo").shortcut_text(
                            ctx.format_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Z)),
                        ),
                    )
                    .clicked()
                {
                    self.undo();
                }
                if ui
                    .add_enabled(
                        can_redo,
                        egui::Button::new("Redo").shortcut_text(
                            ctx.format_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Y)),
                        ),
                    )
                    .clicked()
                {
                    self.redo();
                }
                ui.separator();
                if ui
                    .add_enabled(
                        self.selection != Selection::None,
                        egui::Button::new("Delete selection").shortcut_text("Del"),
                    )
                    .clicked()
                {
                    self.delete_selection();
                }
                ui.separator();
                if ui
                    .add_enabled(self.doc.is_some(), egui::Button::new("Materials…"))
                    .clicked()
                {
                    self.show_materials = true;
                }
                if ui
                    .add_enabled(
                        self.doc.is_some(),
                        egui::Button::new("Prediction settings…"),
                    )
                    .clicked()
                {
                    self.show_settings = true;
                }
            });
            ui.menu_button("View", |ui| {
                if ui
                    .add(egui::Button::new("Fit plan to window").shortcut_text("F"))
                    .clicked()
                {
                    self.fit_pending = true;
                }
                ui.checkbox(&mut self.heat.show, "Show heat map (H)");
                ui.separator();
                let dark = ui.visuals().dark_mode;
                if ui
                    .button(if dark { "Light mode" } else { "Dark mode" })
                    .clicked()
                {
                    ctx.set_theme(if dark {
                        egui::Theme::Light
                    } else {
                        egui::Theme::Dark
                    });
                }
            });
            ui.menu_button("Help", |ui| {
                if ui.button("About Ondalux Heatmap").clicked() {
                    self.show_about = true;
                }
            });
        });
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for t in Tool::ALL {
                if ui.selectable_label(self.tool == t, t.label()).clicked() {
                    self.set_tool(t);
                }
            }
            ui.separator();
            if let Some(doc) = &self.doc {
                ui.label("Material:");
                let current = doc
                    .project
                    .materials
                    .get(self.wall_material)
                    .map_or("—".to_owned(), |m| m.name.clone());
                egui::ComboBox::from_id_salt("wall-material")
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        for m in doc.project.materials.iter() {
                            ui.selectable_value(&mut self.wall_material, m.id, &m.name);
                        }
                    });
                ui.separator();
            }
            ui.checkbox(&mut self.heat.show, "Heat map");
            egui::ComboBox::from_id_salt("heatmap-metric")
                .selected_text(self.heat.metric.label())
                .show_ui(ui, |ui| {
                    for (i, m) in MetricKind::ALL.into_iter().enumerate() {
                        ui.selectable_value(
                            &mut self.heat.metric,
                            m,
                            format!("{}  ({})", m.label(), i + 1),
                        );
                    }
                });
            egui::ComboBox::from_id_salt("heatmap-band")
                .selected_text(self.heat.band.label())
                .show_ui(ui, |ui| {
                    for b in Band::ALL {
                        ui.selectable_value(&mut self.heat.band, b, b.label());
                    }
                });
            ui.add(
                egui::Slider::new(&mut self.heat.opacity, 0.1..=1.0)
                    .text("Opacity")
                    .show_value(false),
            );
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
            {
                if self.worker.busy() {
                    ui.spinner();
                }
                if let Some(t) = self.heat.last_elapsed {
                    ui.weak(format!(
                        "heat map {:.0} ms ({} layers recomputed)",
                        t.as_secs_f64() * 1000.0,
                        self.heat.last_recomputed
                    ));
                }
                if let Some(m) = &self.heat.metrics {
                    ui.weak(format!("coverage {:.0}%", m.coverage_ratio() * 100.0));
                }
                if let Some(text) = self.canvas.hover_text.take() {
                    ui.label(text);
                }
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
            self.heat.metrics = Some(out.metrics);
            self.heat.last_elapsed = Some(out.elapsed);
            self.heat.last_recomputed = out.layers_recomputed;
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(action) = self.pending {
            let mut choice = None;
            egui::Modal::new(egui::Id::new("unsaved")).show(ctx, |ui| {
                ui.heading("Unsaved changes");
                ui.label("The current project has unsaved changes.");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        choice = Some(0);
                    }
                    if ui.button("Discard").clicked() {
                        choice = Some(1);
                    }
                    if ui.button("Cancel").clicked() {
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
            egui::Window::new("About Ondalux Heatmap")
                .open(&mut self.show_about)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(format!("Ondalux Heatmap {}", env!("CARGO_PKG_VERSION")));
                    ui.label("Open source WiFi design and heat map tool.");
                    ui.label("Licensed under MIT or Apache-2.0.");
                });
        }

        panels::calibration_dialog(self, ctx);
        panels::materials_window(self, ctx);
        panels::settings_window(self, ctx);
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

        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        if self.doc.is_some() {
            egui::Panel::right("inspector")
                .resizable(true)
                .default_size(300.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| panels::inspector(self, ui));
                });
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
        ui.add_space(ui.available_height() * 0.3);
        ui.heading("Ondalux Heatmap");
        ui.label("WiFi design and heat maps");
        ui.add_space(16.0);
        if ui.button("New project from floor plan…").clicked() {
            app.guarded(&ctx, Pending::NewFromPlan);
        }
        if ui.button("Open project…").clicked() {
            app.guarded(&ctx, Pending::Open);
        }
    });
}
