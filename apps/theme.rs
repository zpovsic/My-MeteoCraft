//! Colors and global egui styling.

use eframe::egui::{self, Color32, FontId, Stroke, TextStyle, Theme, ThemePreference, Vec2};

pub const BG: Color32 = Color32::from_rgb(13, 18, 29);
pub const CARD: Color32 = Color32::from_rgb(22, 29, 44);
pub const CARD_STROKE: Color32 = Color32::from_rgb(38, 48, 68);
pub const TRACK: Color32 = Color32::from_rgb(34, 43, 62);
pub const HEADER_LEFT: Color32 = Color32::from_rgb(27, 52, 98);
pub const HEADER_RIGHT: Color32 = Color32::from_rgb(17, 24, 39);

pub const TEXT: Color32 = Color32::from_rgb(229, 234, 242);
pub const MUTED: Color32 = Color32::from_rgb(137, 150, 172);

pub const ACCENT: Color32 = Color32::from_rgb(77, 163, 255);
pub const COLD: Color32 = Color32::from_rgb(96, 165, 250);
pub const WARM: Color32 = Color32::from_rgb(251, 146, 60);
pub const GOOD: Color32 = Color32::from_rgb(52, 211, 153);
pub const BAD: Color32 = Color32::from_rgb(248, 113, 113);
pub const HUMID: Color32 = Color32::from_rgb(45, 212, 191);
pub const SNOW: Color32 = Color32::from_rgb(186, 230, 253);

pub fn apply(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = CARD;
    visuals.extreme_bg_color = TRACK;
    visuals.faint_bg_color = Color32::from_rgb(28, 36, 54);
    visuals.hyperlink_color = ACCENT;
    visuals.selection.bg_fill = ACCENT.gamma_multiply(0.4);

    let widgets = &mut visuals.widgets;
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, CARD_STROKE);
    widgets.inactive.weak_bg_fill = Color32::from_rgb(37, 99, 235);
    widgets.inactive.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    widgets.hovered.weak_bg_fill = Color32::from_rgb(59, 130, 246);
    widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    widgets.active.weak_bg_fill = Color32::from_rgb(29, 78, 216);
    for w in [&mut widgets.inactive, &mut widgets.hovered, &mut widgets.active] {
        w.corner_radius = 8.into();
        w.bg_stroke = Stroke::NONE;
    }

    ctx.set_visuals_of(Theme::Dark, visuals);
    ctx.set_theme(ThemePreference::Dark);
    ctx.style_mut_of(Theme::Dark, |style| {
        style.spacing.item_spacing = Vec2::new(12.0, 8.0);
        style.spacing.button_padding = Vec2::new(14.0, 7.0);
        for (text_style, size) in [
            (TextStyle::Small, 12.0),
            (TextStyle::Body, 15.0),
            (TextStyle::Button, 15.0),
            (TextStyle::Heading, 26.0),
        ] {
            style
                .text_styles
                .insert(text_style, FontId::proportional(size));
        }
    });
}

/// Maps a temperature in °C to a color running from icy blue to hot red.
pub fn temperature_color(celsius: f32) -> Color32 {
    const STOPS: [(f32, Color32); 6] = [
        (-15.0, Color32::from_rgb(165, 180, 252)),
        (0.0, Color32::from_rgb(96, 165, 250)),
        (10.0, Color32::from_rgb(45, 212, 191)),
        (20.0, Color32::from_rgb(250, 204, 21)),
        (28.0, Color32::from_rgb(251, 146, 60)),
        (35.0, Color32::from_rgb(239, 68, 68)),
    ];
    let (first, last) = (STOPS[0], STOPS[STOPS.len() - 1]);
    if celsius <= first.0 {
        return first.1;
    }
    STOPS
        .windows(2)
        .find(|w| celsius <= w[1].0)
        .map(|w| {
            let t = (celsius - w[0].0) / (w[1].0 - w[0].0);
            w[0].1.lerp_to_gamma(w[1].1, t)
        })
        .unwrap_or(last.1)
}
