//! Visual style: slightly larger text, more breathing room, rounded
//! widgets and one accent color, in light and dark themes.

use crate::i18n::ThemeChoice;
use eframe::egui::{self, Color32, CornerRadius, FontId, Stroke, TextStyle, Theme};

/// Accent used for selection and primary actions.
pub const ACCENT: Color32 = Color32::from_rgb(37, 99, 235);

pub fn apply(ctx: &egui::Context, theme: ThemeChoice) {
    ctx.set_theme(match theme {
        ThemeChoice::System => egui::ThemePreference::System,
        ThemeChoice::Light => egui::ThemePreference::Light,
        ThemeChoice::Dark => egui::ThemePreference::Dark,
    });
    for t in [Theme::Light, Theme::Dark] {
        ctx.style_mut_of(t, |s| {
            s.text_styles = [
                (TextStyle::Heading, FontId::proportional(19.0)),
                (TextStyle::Body, FontId::proportional(14.5)),
                (TextStyle::Button, FontId::proportional(14.5)),
                (TextStyle::Small, FontId::proportional(12.0)),
                (TextStyle::Monospace, FontId::monospace(13.0)),
            ]
            .into();
            s.spacing.item_spacing = egui::vec2(8.0, 7.0);
            s.spacing.button_padding = egui::vec2(10.0, 5.0);
            s.spacing.interact_size.y = 26.0;
            s.spacing.combo_width = 150.0;
            let r = CornerRadius::same(6);
            let v = &mut s.visuals;
            v.widgets.noninteractive.corner_radius = r;
            v.widgets.inactive.corner_radius = r;
            v.widgets.hovered.corner_radius = r;
            v.widgets.active.corner_radius = r;
            v.widgets.open.corner_radius = r;
            v.window_corner_radius = CornerRadius::same(10);
            v.menu_corner_radius = CornerRadius::same(8);
            v.selection.bg_fill = ACCENT;
            v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
            v.hyperlink_color = ACCENT;
        });
    }
}

/// A large, full-width primary button.
pub fn primary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(text).color(Color32::WHITE).strong())
            .fill(ACCENT)
            .min_size(egui::vec2(ui.available_width().min(320.0), 34.0)),
    )
}

/// Small grey section title.
pub fn section(ui: &mut egui::Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(text.to_uppercase())
            .small()
            .strong()
            .weak(),
    );
}
