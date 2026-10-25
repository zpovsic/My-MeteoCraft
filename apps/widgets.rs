//! Reusable drawing helpers for the dashboard.

use eframe::egui::{
    self, Align2, Color32, FontId, Margin, Mesh, Pos2, Rect, RichText, Sense, Shadow, Shape,
    Stroke, Ui, Vec2,
};

use crate::theme;

pub type CardFn<'a> = Box<dyn FnOnce(&mut Ui) + 'a>;

/// A rounded panel. Returns the height of its content before `min_height` was applied.
pub fn card(ui: &mut Ui, min_height: f32, add_contents: impl FnOnce(&mut Ui)) -> f32 {
    egui::Frame::new()
        .fill(theme::CARD)
        .stroke(Stroke::new(1.0, theme::CARD_STROKE))
        .corner_radius(14)
        .inner_margin(18)
        .shadow(Shadow {
            offset: [0, 4],
            blur: 18,
            spread: 0,
            color: Color32::from_black_alpha(80),
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add_contents(ui);
            let content_height = ui.min_rect().height();
            if min_height > content_height {
                ui.add_space(min_height - content_height - ui.spacing().item_spacing.y);
            }
            content_height
        })
        .inner
}

/// Lays cards out in rows of equal-width columns, as many per row as fit `min_width`.
/// Cards in a row share the height of the tallest one (measured on the previous frame).
pub fn card_rows(ui: &mut Ui, id_salt: &str, min_width: f32, cards: Vec<CardFn<'_>>) {
    let gap = ui.spacing().item_spacing.x;
    let per_row =
        (((ui.available_width() + gap) / (min_width + gap)) as usize).clamp(1, cards.len().max(1));
    let mut cards = cards.into_iter();
    let mut row = 0;
    while cards.len() > 0 {
        let id = ui.make_persistent_id((id_salt, row));
        let previous: f32 = ui.data(|d| d.get_temp(id)).unwrap_or(0.0);
        let chunk: Vec<_> = cards.by_ref().take(per_row).collect();
        let tallest = ui.columns(per_row, |cols| {
            cols.iter_mut()
                .zip(chunk)
                .map(|(col, add)| card(col, previous, add))
                .fold(0.0, f32::max)
        });
        if (tallest - previous).abs() > 0.5 {
            ui.data_mut(|d| d.insert_temp(id, tallest));
            ui.ctx().request_repaint();
        }
        ui.add_space(gap);
        row += 1;
    }
}

pub fn card_title(ui: &mut Ui, icon: &str, title: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(icon).size(17.0).color(theme::ACCENT));
        ui.label(RichText::new(title).size(16.0).strong().color(theme::TEXT));
    });
    ui.add_space(6.0);
}

pub fn caption(ui: &mut Ui, text: &str) {
    ui.label(
        RichText::new(text.to_uppercase())
            .size(11.5)
            .color(theme::MUTED),
    );
}

pub fn muted(text: impl Into<String>) -> RichText {
    RichText::new(text).color(theme::MUTED)
}

/// A small metric tile: icon and caption, a large value and a line of detail.
pub fn metric(ui: &mut Ui, icon: &str, label: &str, accent: Color32, value: &str, detail: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(icon).size(17.0).color(accent));
        caption(ui, label);
    });
    ui.add_space(2.0);
    ui.label(RichText::new(value).size(28.0).color(theme::TEXT));
    ui.label(muted(detail).size(12.5));
}

pub fn status_pill(ui: &mut Ui, text: &str, color: Color32) -> egui::Response {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.16))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.5)))
        .corner_radius(12)
        .inner_margin(Margin::symmetric(10, 3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("●  {text}")).size(12.5).color(color));
            });
        })
        .response
}

/// Paints a left-to-right color gradient.
pub fn horizontal_gradient(painter: &egui::Painter, rect: Rect, left: Color32, right: Color32) {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.left_top(), left);
    mesh.colored_vertex(rect.right_top(), right);
    mesh.colored_vertex(rect.right_bottom(), right);
    mesh.colored_vertex(rect.left_bottom(), left);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(mesh));
}

/// Today's temperature span drawn in temperature colors, with a marker at the current value.
pub fn temperature_range(ui: &mut Ui, min: f32, max: f32, avg: Option<f32>, current: Option<f32>) {
    let height = 44.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    let painter = ui.painter();

    let thickness = 10.0;
    let r = thickness / 2.0;
    let y = rect.top() + 12.0;
    let (x0, x1) = (rect.left() + r, rect.right() - r);
    let span = (max - min).max(0.1);
    let x_of = |v: f32| x0 + (x1 - x0) * ((v - min) / span).clamp(0.0, 1.0);

    painter.circle_filled(Pos2::new(x0, y), r, theme::temperature_color(min));
    painter.circle_filled(Pos2::new(x1, y), r, theme::temperature_color(max));
    const SEGMENTS: usize = 32;
    for i in 0..SEGMENTS {
        let (a, b) = (i as f32 / SEGMENTS as f32, (i + 1) as f32 / SEGMENTS as f32);
        let seg = Rect::from_min_max(
            Pos2::new(x0 + (x1 - x0) * a, y - r),
            Pos2::new(x0 + (x1 - x0) * b, y + r),
        );
        horizontal_gradient(
            painter,
            seg,
            theme::temperature_color(min + span * a),
            theme::temperature_color(min + span * b),
        );
    }

    if let Some(avg) = avg {
        let x = x_of(avg);
        painter.line_segment(
            [Pos2::new(x, y - r - 3.0), Pos2::new(x, y + r + 3.0)],
            Stroke::new(2.0, theme::BG),
        );
    }
    if let Some(cur) = current {
        let p = Pos2::new(x_of(cur), y);
        painter.circle_filled(p, 8.0, Color32::WHITE);
        painter.circle_stroke(p, 8.0, Stroke::new(3.0, theme::temperature_color(cur)));
    }

    let label_y = rect.bottom() - 8.0;
    let font = FontId::proportional(12.5);
    painter.text(
        Pos2::new(rect.left(), label_y),
        Align2::LEFT_CENTER,
        format!("↓ {min:.1} °C"),
        font.clone(),
        theme::COLD,
    );
    painter.text(
        Pos2::new(rect.right(), label_y),
        Align2::RIGHT_CENTER,
        format!("↑ {max:.1} °C"),
        font.clone(),
        theme::WARM,
    );
    if let Some(avg) = avg {
        painter.text(
            Pos2::new(rect.center().x, label_y),
            Align2::CENTER_CENTER,
            format!("avg {avg:.1} °C"),
            font,
            theme::MUTED,
        );
    }
}

/// A compass rose with a needle pointing the way the wind blows, i.e. away from `from_deg`.
pub fn compass(ui: &mut Ui, from_deg: Option<f32>, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let painter = ui.painter();
    let c = rect.center();
    let radius = size / 2.0 - 14.0;

    painter.circle_filled(c, radius, theme::TRACK);
    painter.circle_stroke(c, radius, Stroke::new(1.0, theme::CARD_STROKE));
    for i in 0..16 {
        let deg = i as f32 * 22.5;
        let len = if i % 4 == 0 {
            9.0
        } else if i % 2 == 0 {
            6.0
        } else {
            3.5
        };
        let d = unit(deg);
        painter.line_segment(
            [c + d * (radius - len), c + d * (radius - 1.0)],
            Stroke::new(if i % 4 == 0 { 2.0 } else { 1.0 }, theme::MUTED),
        );
    }
    for (label, deg) in [("N", 0.0_f32), ("E", 90.0), ("S", 180.0), ("W", 270.0)] {
        let color = if label == "N" {
            theme::BAD
        } else {
            theme::MUTED
        };
        painter.text(
            c + unit(deg) * (radius + 8.0),
            Align2::CENTER_CENTER,
            label,
            FontId::proportional(12.0),
            color,
        );
    }

    if let Some(deg) = from_deg {
        let toward = unit(deg + 180.0);
        let side = Vec2::new(-toward.y, toward.x);
        let tip = c + toward * (radius - 12.0);
        let tail = c - toward * (radius - 12.0);
        painter.add(Shape::convex_polygon(
            vec![tip, c + side * 7.0, c - side * 7.0],
            theme::ACCENT,
            Stroke::NONE,
        ));
        painter.add(Shape::convex_polygon(
            vec![tail, c - side * 7.0, c + side * 7.0],
            theme::ACCENT.gamma_multiply(0.35),
            Stroke::NONE,
        ));
    }
    painter.circle_filled(c, 4.0, theme::TEXT);
    painter.circle_filled(c, 2.0, theme::CARD);
}

fn unit(deg: f32) -> Vec2 {
    let rad = deg.to_radians();
    Vec2::new(rad.sin(), -rad.cos())
}
