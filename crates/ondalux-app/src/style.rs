//! Visual design: color palette for light and dark themes, fonts (the
//! operating system's UI font plus Phosphor icons) and the small set of
//! components the panels are built from (cards, buttons, floating bars).

use crate::i18n::ThemeChoice;
use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Margin, RichText, Shadow, Stroke, TextStyle,
    Theme,
};
pub use egui_phosphor::regular as icon;

/// Colors of one theme.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// Behind the floor plan.
    pub backdrop: Color32,
    /// Side panels, header, windows.
    pub panel: Color32,
    /// Cards and buttons at rest.
    pub surface: Color32,
    pub surface_hover: Color32,
    /// Text fields.
    pub input: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,

    /// Background of selected items.
    pub accent_soft: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
}

pub const DARK: Palette = Palette {
    backdrop: Color32::from_rgb(0x0B, 0x0D, 0x12),
    panel: Color32::from_rgb(0x13, 0x16, 0x1C),
    surface: Color32::from_rgb(0x1B, 0x1F, 0x27),
    surface_hover: Color32::from_rgb(0x24, 0x29, 0x33),
    input: Color32::from_rgb(0x0F, 0x12, 0x17),
    border: Color32::from_rgb(0x2A, 0x30, 0x3B),
    text: Color32::from_rgb(0xE8, 0xEB, 0xF0),
    muted: Color32::from_rgb(0x8D, 0x96, 0xA5),
    accent: Color32::from_rgb(0x3B, 0x82, 0xF6),
    accent_soft: Color32::from_rgb(0x1A, 0x2B, 0x4B),
    success: Color32::from_rgb(0x22, 0xC5, 0x5E),
    warning: Color32::from_rgb(0xF5, 0x9E, 0x0B),
    danger: Color32::from_rgb(0xEF, 0x44, 0x44),
};

pub const LIGHT: Palette = Palette {
    backdrop: Color32::from_rgb(0xE9, 0xEC, 0xF1),
    panel: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    surface: Color32::from_rgb(0xF5, 0xF6, 0xF8),
    surface_hover: Color32::from_rgb(0xEA, 0xED, 0xF1),
    input: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    border: Color32::from_rgb(0xDF, 0xE3, 0xE9),
    text: Color32::from_rgb(0x0F, 0x17, 0x2A),
    muted: Color32::from_rgb(0x64, 0x70, 0x84),
    accent: Color32::from_rgb(0x25, 0x63, 0xEB),
    accent_soft: Color32::from_rgb(0xE5, 0xEE, 0xFF),
    success: Color32::from_rgb(0x16, 0xA3, 0x4A),
    warning: Color32::from_rgb(0xD9, 0x77, 0x06),
    danger: Color32::from_rgb(0xDC, 0x26, 0x26),
};

/// Accent color (same hue in both themes), for code that has no `Ui`.
pub const ACCENT: Color32 = Color32::from_rgb(0x3B, 0x82, 0xF6);

/// Palette of the theme `ui` is drawn with.
pub fn palette(ui: &egui::Ui) -> Palette {
    if ui.visuals().dark_mode {
        DARK
    } else {
        LIGHT
    }
}

/// Font family with the semibold face (falls back to the regular one).
pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

/// Text in the semibold face.
pub fn bold(text: impl Into<String>) -> RichText {
    RichText::new(text).family(semibold())
}

pub fn apply(ctx: &egui::Context, theme: ThemeChoice) {
    ctx.set_theme(match theme {
        ThemeChoice::System => egui::ThemePreference::System,
        ThemeChoice::Light => egui::ThemePreference::Light,
        ThemeChoice::Dark => egui::ThemePreference::Dark,
    });
    for (t, p) in [(Theme::Light, LIGHT), (Theme::Dark, DARK)] {
        ctx.style_mut_of(t, |s| style_with(s, p));
    }
}

fn style_with(s: &mut egui::Style, p: Palette) {
    s.text_styles = [
        (TextStyle::Heading, FontId::new(20.0, semibold())),
        (TextStyle::Body, FontId::proportional(14.0)),
        (TextStyle::Button, FontId::proportional(14.0)),
        (TextStyle::Small, FontId::proportional(12.0)),
        (TextStyle::Monospace, FontId::monospace(13.0)),
    ]
    .into();
    let sp = &mut s.spacing;
    sp.item_spacing = egui::vec2(8.0, 8.0);
    sp.button_padding = egui::vec2(12.0, 6.0);
    sp.interact_size.y = 30.0;
    sp.window_margin = Margin::same(16);
    sp.menu_margin = Margin::same(6);
    sp.combo_width = 160.0;
    sp.icon_spacing = 6.0;
    sp.slider_rail_height = 4.0;

    let v = &mut s.visuals;
    v.panel_fill = p.panel;
    v.window_fill = p.panel;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.extreme_bg_color = p.input;
    v.text_edit_bg_color = Some(p.input);
    v.faint_bg_color = p.surface;
    v.code_bg_color = p.surface;
    v.hyperlink_color = p.accent;
    v.warn_fg_color = p.warning;
    v.error_fg_color = p.danger;
    v.override_text_color = None;
    v.selection.bg_fill = p.accent_soft;
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.slider_trailing_fill = true;

    let r = CornerRadius::same(8);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.panel;
    w.noninteractive.weak_bg_fill = p.panel;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.bg_fill = p.surface;
    w.inactive.weak_bg_fill = p.surface;
    w.inactive.bg_stroke = Stroke::new(1.0, p.border);
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.hovered.bg_fill = p.surface_hover;
    w.hovered.weak_bg_fill = p.surface_hover;
    w.hovered.bg_stroke = Stroke::new(1.0, p.border.gamma_multiply(1.4));
    w.hovered.fg_stroke = Stroke::new(1.5, p.text);
    w.active.bg_fill = p.accent_soft;
    w.active.weak_bg_fill = p.accent_soft;
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    // Also the color of `ui.strong()` text.
    w.active.fg_stroke = Stroke::new(1.5, p.text);
    w.open.bg_fill = p.surface_hover;
    w.open.weak_bg_fill = p.surface_hover;
    w.open.bg_stroke = Stroke::new(1.0, p.border);
    w.open.fg_stroke = Stroke::new(1.0, p.text);
    for wv in [
        &mut w.noninteractive,
        &mut w.inactive,
        &mut w.hovered,
        &mut w.active,
        &mut w.open,
    ] {
        wv.corner_radius = r;
        wv.expansion = 0.0;
    }
    let shadow_alpha = if v.dark_mode { 90 } else { 28 };
    v.window_corner_radius = CornerRadius::same(14);
    v.menu_corner_radius = CornerRadius::same(10);
    v.window_shadow = Shadow {
        offset: [0, 10],
        blur: 30,
        spread: 0,
        color: Color32::from_black_alpha(shadow_alpha),
    };
    v.popup_shadow = Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(shadow_alpha),
    };
}

/// System UI font (Segoe UI, San Francisco, or a common Linux sans), a
/// semibold face, and the Phosphor icon font. Falls back to egui's fonts.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let regular_candidates = [
        "C:\\Windows\\Fonts\\segoeui.ttf",
        "/System/Library/Fonts/SFNS.ttf",
        "/usr/share/fonts/truetype/inter/Inter-Regular.ttf",
        "/usr/share/fonts/opentype/inter/Inter-Regular.otf",
        "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/google-noto/NotoSans-Regular.ttf",
    ];
    let semibold_candidates = [
        "C:\\Windows\\Fonts\\seguisb.ttf",
        "/usr/share/fonts/truetype/inter/Inter-SemiBold.ttf",
        "/usr/share/fonts/opentype/inter/Inter-SemiBold.otf",
        "/usr/share/fonts/truetype/noto/NotoSans-SemiBold.ttf",
        "/usr/share/fonts/noto/NotoSans-SemiBold.ttf",
        "/usr/share/fonts/google-noto/NotoSans-SemiBold.ttf",
    ];
    let load = |paths: &[&str]| paths.iter().find_map(|p| std::fs::read(p).ok());
    let regular = load(&regular_candidates);
    let semibold_bytes = load(&semibold_candidates);
    let proportional = fonts.families.entry(FontFamily::Proportional).or_default();
    if let Some(bytes) = regular {
        fonts
            .font_data
            .insert("ui".into(), egui::FontData::from_owned(bytes).into());
        proportional.insert(0, "ui".into());
    }
    let mut semi = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    if let Some(bytes) = semibold_bytes {
        fonts.font_data.insert(
            "ui-semibold".into(),
            egui::FontData::from_owned(bytes).into(),
        );
        semi.insert(0, "ui-semibold".into());
    }
    fonts.families.insert(semibold(), semi);
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    // Icons also inside semibold text.
    if let Some(f) = fonts.families.get_mut(&semibold()) {
        f.insert(1.min(f.len()), "phosphor".into());
    }
    ctx.set_fonts(fonts);
}

// ---------------------------------------------------------------- components

/// A rounded card with a subtle border.
pub fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let p = palette(ui);
    egui::Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(10)
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// Frame for toolbars floating over the floor plan.
pub fn floating(ui: &egui::Ui) -> egui::Frame {
    let p = palette(ui);
    egui::Frame::new()
        .fill(p.panel)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(12)
        .inner_margin(Margin::symmetric(8, 6))
        .shadow(ui.visuals().popup_shadow)
}

/// Large accent button, full width (up to 340 px).
pub fn primary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let p = palette(ui);
    ui.add(
        egui::Button::new(bold(text).color(Color32::WHITE).size(14.5))
            .fill(p.accent)
            .stroke(Stroke::NONE)
            .corner_radius(9)
            .min_size(egui::vec2(ui.available_width().min(340.0), 38.0)),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Destructive action, drawn in the danger color.
pub fn danger_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let p = palette(ui);
    ui.add(egui::Button::new(RichText::new(text).color(p.danger)).frame_when_inactive(false))
}

/// Square, frameless icon button with a tooltip.
pub fn icon_button(ui: &mut egui::Ui, glyph: &str, tooltip: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(glyph).size(17.0))
            .frame_when_inactive(false)
            .min_size(egui::vec2(32.0, 32.0)),
    )
    .on_hover_text(tooltip)
}

/// Full-width tool entry: icon, label and the keyboard shortcut.
pub fn tool_button(
    ui: &mut egui::Ui,
    glyph: &str,
    label: &str,
    key: &str,
    selected: bool,
) -> egui::Response {
    let p = palette(ui);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 38.0), egui::Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if ui.is_rect_visible(rect) {
        let (fill, fg) = if selected {
            (p.accent_soft, p.accent)
        } else if response.hovered() {
            (p.surface_hover, p.text)
        } else {
            (Color32::TRANSPARENT, p.text)
        };
        let painter = ui.painter();
        painter.rect_filled(rect, 9.0, fill);
        if selected {
            let bar = egui::Rect::from_min_size(
                rect.left_top() + egui::vec2(0.0, 9.0),
                egui::vec2(3.0, rect.height() - 18.0),
            );
            painter.rect_filled(bar, 2.0, p.accent);
        }
        painter.text(
            rect.left_center() + egui::vec2(14.0, 0.0),
            egui::Align2::LEFT_CENTER,
            glyph,
            FontId::proportional(18.0),
            if selected { p.accent } else { p.muted },
        );
        let font = if selected {
            FontId::new(14.0, semibold())
        } else {
            FontId::proportional(14.0)
        };
        painter.text(
            rect.left_center() + egui::vec2(42.0, 0.0),
            egui::Align2::LEFT_CENTER,
            label,
            font,
            fg,
        );
        // Shortcut key in a small keycap (none for an empty key).
        if key.is_empty() {
            return response;
        }
        let key_rect = egui::Rect::from_center_size(
            rect.right_center() - egui::vec2(18.0, 0.0),
            egui::vec2(20.0, 20.0),
        );
        painter.rect_stroke(
            key_rect,
            5.0,
            Stroke::new(1.0, p.border),
            egui::StrokeKind::Inside,
        );
        painter.text(
            key_rect.center(),
            egui::Align2::CENTER_CENTER,
            key,
            FontId::proportional(11.0),
            p.muted,
        );
    }
    response
}

/// Small muted uppercase section title.
pub fn section(ui: &mut egui::Ui, text: &str) {
    let p = palette(ui);
    ui.add_space(8.0);
    ui.label(
        RichText::new(text.to_uppercase())
            .size(11.0)
            .family(semibold())
            .color(p.muted)
            .extra_letter_spacing(0.6),
    );
    ui.add_space(2.0);
}

/// A rounded pill with colored text, e.g. a status.
pub fn badge(ui: &mut egui::Ui, text: &str, color: Color32) -> egui::Response {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.16))
        .corner_radius(20)
        .inner_margin(Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(bold(text).size(12.0).color(color));
        })
        .response
}

/// Big number with a caption, for summaries.
pub fn stat(ui: &mut egui::Ui, value: &str, caption: &str, color: Color32) {
    let p = palette(ui);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.label(bold(value).size(22.0).color(color));
        ui.label(RichText::new(caption).size(12.0).color(p.muted));
    });
}
