//! The last 24 hours as charts: one card per measurement, sharing a time axis and a
//! hover line.

use eframe::egui::{
    self, Align, Align2, Color32, FontId, Layout, Mesh, Pos2, Rect, RichText, Sense, Shape, Stroke,
    Ui, Vec2,
};

use crate::history::{History, Point};
use crate::{theme, widgets};

const CHART_HEIGHT: f32 = 190.0;
/// Room for the value labels on the left and the hour labels below.
const LEFT_MARGIN: f32 = 46.0;
const BOTTOM_MARGIN: f32 = 22.0;
/// Points further apart than this are drawn with a gap between them.
const GAP_MINUTES: f64 = 10.0;
const DAY: f64 = 24.0 * 60.0;
const PRESSURE_COLOR: Color32 = Color32::from_rgb(192, 132, 252);

struct Chart<'a> {
    icon: &'static str,
    title: &'static str,
    unit: &'static str,
    decimals: usize,
    /// The smallest value range shown, so a steady value doesn't look like noise.
    min_span: f32,
    points: &'a [Point],
    /// `None` colors the line by temperature.
    color: Option<Color32>,
    /// Further series drawn as thin lines where they have values.
    extras: Vec<(&'static str, Color32, &'a [Point])>,
}

pub fn show(ui: &mut Ui, history: &History) {
    let (Some(first), Some(last)) = (history.temperature.first(), history.temperature.last())
    else {
        return;
    };
    let span = (first.minutes, last.minutes.max(first.minutes + 1.0));
    let hover_id = ui.make_persistent_id("chart-hover");
    let shared: Option<f64> = ui.data(|d| d.get_temp(hover_id));
    let mut hovered = None;
    for chart in charts(history) {
        widgets::card(ui, 0.0, |ui| {
            if let Some(minutes) = chart.ui(ui, span, shared) {
                hovered = Some(minutes);
            }
        });
        ui.add_space(ui.spacing().item_spacing.x);
    }
    if hovered != shared {
        ui.data_mut(|d| match hovered {
            Some(minutes) => {
                d.insert_temp(hover_id, minutes);
            }
            None => d.remove::<f64>(hover_id),
        });
        ui.ctx().request_repaint();
    }
}

fn charts(history: &History) -> Vec<Chart<'_>> {
    let extras = [
        ("Wind chill", theme::SNOW, &history.wind_chill),
        ("Heat index", theme::WARM, &history.heat_index),
    ];
    vec![
        Chart {
            icon: "🌡",
            title: "Temperature",
            unit: "°C",
            decimals: 1,
            min_span: 3.0,
            points: &history.temperature,
            color: None,
            extras: extras
                .into_iter()
                .filter(|(_, _, points)| !points.is_empty())
                .map(|(name, color, points)| (name, color, points.as_slice()))
                .collect(),
        },
        Chart {
            icon: "⏲",
            title: "Air pressure",
            unit: "hPa",
            decimals: 1,
            min_span: 6.0,
            points: &history.pressure,
            color: Some(PRESSURE_COLOR),
            extras: Vec::new(),
        },
        Chart {
            icon: "💧",
            title: "Humidity",
            unit: "%",
            decimals: 0,
            min_span: 10.0,
            points: &history.humidity,
            color: Some(theme::HUMID),
            extras: Vec::new(),
        },
    ]
}

impl Chart<'_> {
    fn color_of(&self, value: f32) -> Color32 {
        self.color
            .unwrap_or_else(|| theme::temperature_color(value))
    }

    fn format(&self, value: f32) -> String {
        format!("{value:.*} {}", self.decimals, self.unit)
    }

    /// Draws the chart; returns the time under the pointer when it is over the plot.
    fn ui(&self, ui: &mut Ui, span: (f64, f64), shared_hover: Option<f64>) -> Option<f64> {
        let extremes = extremes(self.points);
        self.header(ui, extremes);

        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), CHART_HEIGHT),
            Sense::hover(),
        );
        let plot = Rect::from_min_max(
            Pos2::new(rect.left() + LEFT_MARGIN, rect.top() + 6.0),
            Pos2::new(rect.right() - 8.0, rect.bottom() - BOTTOM_MARGIN),
        );
        let Some((min, max)) = extremes else {
            ui.painter().text(
                plot.center(),
                Align2::CENTER_CENTER,
                "No data in the station's graph",
                FontId::proportional(13.0),
                theme::MUTED,
            );
            return None;
        };

        let all = self
            .extras
            .iter()
            .flat_map(|(_, _, points)| points.iter())
            .chain(self.points);
        let (low, high) = all.fold((min.value, max.value), |(lo, hi), p| {
            (lo.min(p.value), hi.max(p.value))
        });
        let axis = ValueAxis::new(low, high, self.min_span);
        let x_of = |minutes: f64| {
            plot.left() + ((minutes - span.0) / (span.1 - span.0)) as f32 * plot.width()
        };
        let y_of = |value: f32| {
            plot.bottom() - (value - axis.low) / (axis.high - axis.low) * plot.height()
        };
        let painter = ui.painter_at(rect);

        // Grid: values on the left, every third hour below.
        let label_font = FontId::proportional(11.5);
        for tick in axis.ticks() {
            let y = y_of(tick);
            painter.line_segment(
                [Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
                Stroke::new(1.0, theme::CARD_STROKE),
            );
            painter.text(
                Pos2::new(plot.left() - 8.0, y),
                Align2::RIGHT_CENTER,
                format!("{tick:.*}", axis.decimals),
                label_font.clone(),
                theme::MUTED,
            );
        }
        let mut hour = (span.0 / 180.0).ceil() * 180.0;
        while hour <= span.1 {
            let x = x_of(hour);
            painter.line_segment(
                [Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())],
                Stroke::new(1.0, theme::CARD_STROKE.gamma_multiply(0.6)),
            );
            if x - plot.left() > 14.0 && plot.right() - x > 14.0 {
                painter.text(
                    Pos2::new(x, plot.bottom() + 5.0),
                    Align2::CENTER_TOP,
                    clock(hour),
                    label_font.clone(),
                    theme::MUTED,
                );
            }
            hour += 180.0;
        }

        // The area under the line fades out toward the bottom.
        for run in runs(self.points) {
            let mut mesh = Mesh::default();
            for (i, point) in run.iter().enumerate() {
                let x = x_of(point.minutes);
                let color = self.color_of(point.value);
                mesh.colored_vertex(Pos2::new(x, y_of(point.value)), color.gamma_multiply(0.28));
                mesh.colored_vertex(Pos2::new(x, plot.bottom()), color.gamma_multiply(0.0));
                if i > 0 {
                    let i = i as u32 * 2;
                    mesh.add_triangle(i - 2, i - 1, i);
                    mesh.add_triangle(i - 1, i, i + 1);
                }
            }
            painter.add(Shape::mesh(mesh));
        }
        for (_, color, points) in &self.extras {
            for run in runs(points) {
                let line: Vec<Pos2> = run
                    .iter()
                    .map(|p| Pos2::new(x_of(p.minutes), y_of(p.value)))
                    .collect();
                painter.add(Shape::line(line, Stroke::new(1.6, *color)));
            }
        }
        for run in runs(self.points) {
            let line: Vec<Pos2> = run
                .iter()
                .map(|p| Pos2::new(x_of(p.minutes), y_of(p.value)))
                .collect();
            match self.color {
                Some(color) => {
                    painter.add(Shape::line(line, Stroke::new(2.2, color)));
                }
                None => {
                    for (pair, points) in line.windows(2).zip(run.windows(2)) {
                        let color = self.color_of((points[0].value + points[1].value) / 2.0);
                        painter.line_segment([pair[0], pair[1]], Stroke::new(2.2, color));
                    }
                }
            }
        }

        // The lowest and highest values, and the latest one.
        for (point, above) in [(max, true), (min, false)] {
            let at = Pos2::new(x_of(point.minutes), y_of(point.value));
            painter.circle_filled(at, 3.5, self.color_of(point.value));
            painter.circle_stroke(at, 3.5, Stroke::new(1.5, theme::CARD));
            let (offset, align) = if above {
                (-7.0, Align2::CENTER_BOTTOM)
            } else {
                (7.0, Align2::CENTER_TOP)
            };
            painter.text(
                Pos2::new(
                    at.x.clamp(plot.left() + 16.0, plot.right() - 16.0),
                    at.y + offset,
                ),
                align,
                format!("{:.*}", self.decimals, point.value),
                FontId::proportional(11.5),
                theme::TEXT,
            );
        }
        if let Some(last) = self.points.last() {
            let at = Pos2::new(x_of(last.minutes), y_of(last.value));
            let color = self.color_of(last.value);
            painter.circle_filled(at, 7.0, color.gamma_multiply(0.25));
            painter.circle_filled(at, 4.0, color);
            painter.circle_stroke(at, 4.0, Stroke::new(1.5, Color32::WHITE));
        }

        let pointer = response
            .hover_pos()
            .filter(|pos| pos.x >= plot.left() && pos.x <= plot.right())
            .map(|pos| {
                span.0 + f64::from((pos.x - plot.left()) / plot.width()) * (span.1 - span.0)
            });
        if let Some(minutes) = pointer.or(shared_hover) {
            self.hover(&painter, plot, minutes, &x_of, &y_of);
        }
        pointer
    }

    fn header(&self, ui: &mut Ui, extremes: Option<(Point, Point)>) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(self.icon).size(17.0).color(theme::ACCENT));
            ui.label(
                RichText::new(self.title)
                    .size(16.0)
                    .strong()
                    .color(theme::TEXT),
            );
            for (name, color, _) in &self.extras {
                ui.add_space(6.0);
                let (swatch, _) = ui.allocate_exact_size(Vec2::new(16.0, 10.0), Sense::hover());
                ui.painter().line_segment(
                    [swatch.left_center(), swatch.right_center()],
                    Stroke::new(2.0, *color),
                );
                ui.label(RichText::new(*name).size(12.5).color(*color));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some(last) = self.points.last() {
                    ui.label(
                        RichText::new(self.format(last.value))
                            .size(22.0)
                            .strong()
                            .color(self.color_of(last.value)),
                    );
                }
            });
        });
        if let Some((min, max)) = extremes {
            ui.label(
                widgets::muted(format!(
                    "Low {} at {} · high {} at {}",
                    self.format(min.value),
                    clock(min.minutes),
                    self.format(max.value),
                    clock(max.minutes),
                ))
                .size(12.5),
            );
        }
        ui.add_space(4.0);
    }

    fn hover(
        &self,
        painter: &egui::Painter,
        plot: Rect,
        minutes: f64,
        x_of: &dyn Fn(f64) -> f32,
        y_of: &dyn Fn(f32) -> f32,
    ) {
        let Some(point) = nearest(self.points, minutes, GAP_MINUTES) else {
            return;
        };
        let x = x_of(point.minutes);
        painter.line_segment(
            [Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())],
            Stroke::new(1.0, theme::TEXT.gamma_multiply(0.35)),
        );
        let at = Pos2::new(x, y_of(point.value));
        let color = self.color_of(point.value);
        painter.circle_filled(at, 5.0, Color32::WHITE);
        painter.circle_stroke(at, 5.0, Stroke::new(2.5, color));

        let mut text = format!("{}   {}", clock(point.minutes), self.format(point.value));
        for (name, _, points) in &self.extras {
            if let Some(extra) = nearest(points, point.minutes, 3.0) {
                text.push_str(&format!("\n{name} {}", self.format(extra.value)));
            }
        }
        let galley = painter.layout_no_wrap(text, FontId::proportional(13.0), theme::TEXT);
        let size = galley.size() + Vec2::new(16.0, 10.0);
        let mut min = Pos2::new(x + 12.0, plot.top() + 4.0);
        if min.x + size.x > plot.right() {
            min.x = x - 12.0 - size.x;
        }
        let tip = Rect::from_min_size(min, size);
        painter.rect(
            tip,
            6,
            theme::BG.gamma_multiply(0.92),
            Stroke::new(1.0, theme::CARD_STROKE),
            egui::StrokeKind::Inside,
        );
        painter.galley(tip.min + Vec2::new(8.0, 5.0), galley, theme::TEXT);
    }
}

/// A value range with round grid lines.
struct ValueAxis {
    low: f32,
    high: f32,
    step: f32,
    decimals: usize,
}

impl ValueAxis {
    fn new(low: f32, high: f32, min_span: f32) -> Self {
        let center = (low + high) / 2.0;
        let span = (high - low).max(min_span);
        let (low, high) = (center - span * 0.55, center + span * 0.55);
        let rough = (high - low) / 4.0;
        let magnitude = 10_f32.powf(rough.log10().floor());
        let step = [1.0, 2.0, 2.5, 5.0, 10.0]
            .into_iter()
            .map(|m| m * magnitude)
            .find(|&step| step >= rough)
            .unwrap_or(10.0 * magnitude);
        let decimals = (0..3)
            .find(|&d| {
                let scaled = step * 10_f32.powi(d);
                (scaled - scaled.round()).abs() < 1e-3
            })
            .unwrap_or(3) as usize;
        Self {
            low: (low / step).floor() * step,
            high: (high / step).ceil() * step,
            step,
            decimals,
        }
    }

    fn ticks(&self) -> impl Iterator<Item = f32> + '_ {
        let count = ((self.high - self.low) / self.step).round() as i32;
        (0..=count).map(move |i| self.low + i as f32 * self.step)
    }
}

/// Splits a series where it has gaps.
fn runs(points: &[Point]) -> impl Iterator<Item = &[Point]> {
    points.chunk_by(|a, b| b.minutes - a.minutes <= GAP_MINUTES)
}

fn extremes(points: &[Point]) -> Option<(Point, Point)> {
    let min = points.iter().min_by(|a, b| a.value.total_cmp(&b.value))?;
    let max = points.iter().max_by(|a, b| a.value.total_cmp(&b.value))?;
    Some((*min, *max))
}

/// The point closest in time to `minutes`, if one is within `within` minutes.
fn nearest(points: &[Point], minutes: f64, within: f64) -> Option<Point> {
    let i = points.partition_point(|p| p.minutes < minutes);
    [i.checked_sub(1), Some(i)]
        .into_iter()
        .flatten()
        .filter_map(|i| points.get(i))
        .min_by(|a, b| {
            (a.minutes - minutes)
                .abs()
                .total_cmp(&(b.minutes - minutes).abs())
        })
        .filter(|p| (p.minutes - minutes).abs() <= within)
        .copied()
}

/// "HH:MM" for minutes past the first midnight.
pub fn clock(minutes: f64) -> String {
    let minutes = minutes.rem_euclid(DAY).round() as u32 % (24 * 60);
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_wraps_past_midnight() {
        assert_eq!(clock(600.0), "10:00");
        assert_eq!(clock(DAY + 536.4), "08:56");
        assert_eq!(clock(DAY - 0.2), "00:00");
    }

    #[test]
    fn value_axis_has_round_ticks_around_the_data() {
        let axis = ValueAxis::new(0.6, 8.8, 3.0);
        assert!(axis.low <= 0.6 && axis.high >= 8.8);
        assert_eq!(axis.step, 2.5);
        assert_eq!(axis.decimals, 1);
        let steady = ValueAxis::new(1022.0, 1022.4, 6.0);
        assert!(steady.high - steady.low >= 6.0);
        assert_eq!(steady.decimals, 0);
    }

    #[test]
    fn nearest_point_respects_gaps() {
        let points = [
            Point {
                minutes: 0.0,
                value: 1.0,
            },
            Point {
                minutes: 2.0,
                value: 2.0,
            },
            Point {
                minutes: 30.0,
                value: 3.0,
            },
        ];
        assert_eq!(nearest(&points, 1.4, 5.0).unwrap().value, 2.0);
        assert!(nearest(&points, 16.0, 5.0).is_none());
        assert_eq!(runs(&points).count(), 2);
    }
}
