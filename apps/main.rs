#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod station;

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, FontId, RichText, Sense, Stroke, Vec2};

use station::{FetchError, StationData};

/// The station page itself refreshes every 5 minutes.
const REFRESH_INTERVAL: Duration = Duration::from_secs(300);

const ACCENT: Color32 = Color32::from_rgb(64, 156, 255);

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("MeteoCraft – Ljubljana Koseze")
            .with_inner_size([1100.0, 820.0])
            .with_min_inner_size([700.0, 500.0]),
        ..Default::default()
    };
    eframe::run_native(
        "MeteoCraft",
        options,
        Box::new(|cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            Ok(Box::new(MeteoApp::new(cc.egui_ctx.clone())))
        }),
    )
}

struct MeteoApp {
    ctx: egui::Context,
    data: Option<StationData>,
    error: Option<String>,
    pending: Option<Receiver<Result<StationData, FetchError>>>,
    last_fetch: Instant,
}

impl MeteoApp {
    fn new(ctx: egui::Context) -> Self {
        let mut app = Self {
            ctx,
            data: None,
            error: None,
            pending: None,
            last_fetch: Instant::now(),
        };
        app.refresh();
        app
    }

    fn refresh(&mut self) {
        if self.pending.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(station::fetch());
            ctx.request_repaint();
        });
        self.pending = Some(rx);
        self.last_fetch = Instant::now();
    }

    fn poll_fetch(&mut self) {
        let Some(rx) = &self.pending else { return };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Err(FetchError::Http("download thread stopped unexpectedly".into()))
            }
        };
        self.pending = None;
        match result {
            Ok(data) => {
                self.data = Some(data);
                self.error = None;
                // Live camera and graphs are regenerated on the server alongside the data.
                self.ctx.forget_all_images();
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.heading(RichText::new("Ljubljana Koseze Weather Station").strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let loading = self.pending.is_some();
                if ui
                    .add_enabled(!loading, egui::Button::new("⟳ Refresh"))
                    .clicked()
                {
                    self.refresh();
                }
                if loading {
                    ui.spinner();
                } else {
                    let left = REFRESH_INTERVAL.saturating_sub(self.last_fetch.elapsed());
                    ui.weak(format!(
                        "next update in {}:{:02}",
                        left.as_secs() / 60,
                        left.as_secs() % 60
                    ));
                }
            });
        });
        ui.horizontal(|ui| {
            if let Some(data) = &self.data {
                ui.label("Last measurement:");
                ui.strong(data.last_measurement.replace(" ob ", " at "));
                ui.separator();
            }
            ui.hyperlink_to("vreme.zajcek.org/koseze", station::STATION_URL);
        });
        if let Some(err) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, format!("⚠ {err}"));
        }
        ui.add_space(6.0);
    }
}

impl eframe::App for MeteoApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_fetch();
        let elapsed = self.last_fetch.elapsed();
        if elapsed >= REFRESH_INTERVAL {
            self.refresh();
        }
        // Repaint once a second so the countdown stays current.
        ctx.request_repaint_after(Duration::from_secs(1));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("header").show(ui, |ui| self.header(ui));

        egui::Panel::bottom("footer").show(ui, |ui| {
            ui.add_space(4.0);
            ui.small(
                "Unofficial data from a private LaCrosse WS2300 station (not part of the ARSO network). \
                 For information only.",
            );
            ui.add_space(4.0);
        });

        egui::CentralPanel::default().show(ui, |ui| {
            let Some(data) = &self.data else {
                ui.centered_and_justified(|ui| {
                    if self.pending.is_some() {
                        ui.spinner();
                    } else {
                        ui.label("No data available.");
                    }
                });
                return;
            };
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| dashboard(ui, data));
        });
    }
}

fn dashboard(ui: &mut egui::Ui, data: &StationData) {
    ui.columns(3, |cols| {
        card(&mut cols[0], "Atmosphere", |ui| atmosphere(ui, data));
        card(&mut cols[1], "Wind", |ui| wind(ui, &data.wind));
        card(&mut cols[2], "Precipitation", |ui| {
            readings_grid(ui, "precipitation", &data.precipitation)
        });
    });

    ui.add_space(10.0);
    card(ui, "Today's Statistics", |ui| daily_stats(ui, data));

    ui.add_space(10.0);
    card(ui, "All-Time Records", |ui| records(ui, data));

    ui.add_space(10.0);
    ui.columns(3, |cols| {
        card(&mut cols[0], "Live Camera (facing SW)", |ui| {
            remote_image(ui, "livefeed/live.jpg")
        });
        card(&mut cols[1], "Temperature and Wind Chill", |ui| {
            remote_image(ui, "graf.png")
        });
        card(&mut cols[2], "Air Pressure", |ui| remote_image(ui, "graf_tlak.png"));
    });
    ui.add_space(10.0);
}

fn card(ui: &mut egui::Ui, title: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::group(ui.style())
        .inner_margin(12.0)
        .corner_radius(8.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).strong().size(16.0).color(ACCENT));
            ui.add_space(6.0);
            add_contents(ui);
        });
}

fn atmosphere(ui: &mut egui::Ui, data: &StationData) {
    if let Some(temp) = data.reading("temperatura") {
        ui.label(RichText::new(&temp.value).size(40.0).strong());
        ui.add_space(4.0);
    }
    let rest: Vec<_> = data
        .atmosphere
        .iter()
        .filter(|r| !r.label.starts_with("temperatura"))
        .cloned()
        .collect();
    readings_grid(ui, "atmosphere", &rest);
}

fn readings_grid(ui: &mut egui::Ui, id: &str, readings: &[station::Reading]) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([16.0, 6.0])
        .show(ui, |ui| {
            for r in readings {
                ui.label(english(&r.label));
                ui.strong(&r.value);
                ui.end_row();
            }
        });
}

fn wind(ui: &mut egui::Ui, wind: &station::Wind) {
    ui.horizontal(|ui| {
        compass(ui, wind.direction_deg);
        ui.vertical(|ui| {
            ui.label("speed");
            ui.strong(&wind.speed);
            ui.add_space(6.0);
            ui.label("direction");
            // The site writes e.g. "WNW oz. 292.5 °" ("oz." = "or").
            let compass = wind.direction.split_whitespace().next().unwrap_or_default();
            match wind.direction_deg {
                Some(deg) => ui.strong(format!("{compass} ({deg}°)")),
                None => ui.strong(&wind.direction),
            };
        });
    });
}

/// Draws a compass rose with an arrow blowing *from* the reported direction.
fn compass(ui: &mut egui::Ui, direction_deg: Option<f32>) {
    let (response, painter) = ui.allocate_painter(Vec2::splat(120.0), Sense::hover());
    let center = response.rect.center();
    let radius = 46.0;
    let visuals = ui.visuals();
    let fg = visuals.text_color();

    painter.circle_stroke(center, radius, Stroke::new(1.5, visuals.weak_text_color()));
    for (label, deg) in [("N", 0.0_f32), ("E", 90.0), ("S", 180.0), ("W", 270.0)] {
        let dir = unit(deg);
        painter.text(
            center + dir * (radius + 9.0),
            Align2::CENTER_CENTER,
            label,
            FontId::proportional(12.0),
            fg,
        );
    }

    if let Some(deg) = direction_deg {
        let from = center + unit(deg) * (radius - 4.0);
        painter.arrow(from, center - from, Stroke::new(3.0, ACCENT));
    }
    painter.circle_filled(center, 3.0, fg);
}

fn unit(deg: f32) -> Vec2 {
    let rad = deg.to_radians();
    Vec2::new(rad.sin(), -rad.cos())
}

fn daily_stats(ui: &mut egui::Ui, data: &StationData) {
    egui::Grid::new("daily")
        .num_columns(4)
        .striped(true)
        .spacing([32.0, 6.0])
        .min_col_width(110.0)
        .show(ui, |ui| {
            ui.label("");
            for title in ["minimum", "maximum", "average"] {
                ui.strong(title);
            }
            ui.end_row();
            for stat in &data.daily {
                ui.label(english(&stat.label));
                for value in [&stat.min, &stat.max, &stat.avg] {
                    ui.label(value.as_deref().unwrap_or("—"));
                }
                ui.end_row();
            }
        });
}

fn records(ui: &mut egui::Ui, data: &StationData) {
    egui::Grid::new("records")
        .num_columns(3)
        .striped(true)
        .spacing([32.0, 6.0])
        .min_col_width(110.0)
        .show(ui, |ui| {
            ui.label("");
            ui.strong("minimum");
            ui.strong("maximum");
            ui.end_row();
            for record in &data.records {
                ui.label(english(&record.label));
                for extreme in [&record.min, &record.max] {
                    match extreme {
                        Some(e) => {
                            ui.horizontal(|ui| {
                                ui.strong(&e.value);
                                ui.weak(&e.when);
                            });
                        }
                        None => {
                            ui.label("—");
                        }
                    }
                }
                ui.end_row();
            }
        });
}

/// English names for the Slovenian labels used on the station page.
/// Unknown labels are shown as-is.
fn english(label: &str) -> &str {
    match label {
        "temperatura" => "temperature",
        "zračni tlak" | "tlak" => "air pressure",
        "vlažnost zraka" | "vlaga" => "humidity",
        "rosišče" => "dew point",
        "občutek mraza" => "wind chill",
        "toplotni indeks" => "heat index",
        "hitrost vetra" => "wind speed",
        "sunek vetra" => "wind gust",
        "padavine v zadnji uri" => "precipitation, last hour",
        "padavine zadnjih 24h" => "precipitation, last 24 h",
        "padavine v 1h" => "precipitation in 1 h",
        "padavine v 24h" => "precipitation in 24 h",
        "višina snežne odeje" => "snow depth",
        "najzgodnejše sneženje in snežna odeja" => "earliest snowfall and snow cover",
        "najpoznejše sneženje in snežne odeja" => "latest snowfall and snow cover",
        other => other,
    }
}

fn remote_image(ui: &mut egui::Ui, path: &str) {
    let url = station::image_url(path);
    // Inside a scroll area the available height is not meaningful, so fit by width
    // (all station images are wider than they are tall).
    let image = egui::Image::new(&url)
        .fit_to_exact_size(Vec2::splat(ui.available_width()))
        .corner_radius(4.0)
        .show_loading_spinner(true);
    let response = ui.add(image.sense(Sense::click()));
    if response.on_hover_text("Open in browser").clicked() {
        ui.ctx().open_url(egui::OpenUrl::new_tab(url));
    }
}

#[cfg(test)]
mod tests {
    use super::english;

    #[test]
    fn every_label_on_the_page_has_an_english_name() {
        let data = crate::station::parse(include_str!("../tests/fixtures/koseze.html")).unwrap();
        let labels = data
            .atmosphere
            .iter()
            .chain(&data.precipitation)
            .map(|r| &r.label)
            .chain(data.daily.iter().map(|s| &s.label))
            .chain(data.records.iter().map(|r| &r.label));
        for label in labels {
            assert_ne!(english(label), label, "no translation for {label:?}");
        }
    }
}
