#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod station;
mod theme;
mod widgets;

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use eframe::egui::{self, Margin, RichText, Sense, Stroke, Vec2};

use station::{FetchError, StationData};
use widgets::{CardFn, caption, card_title, muted};

/// The station page itself refreshes every 5 minutes.
const REFRESH_INTERVAL: Duration = Duration::from_secs(300);

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("MeteoCraft – Ljubljana Koseze")
            .with_inner_size([1180.0, 880.0])
            .with_min_inner_size([540.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native(
        "MeteoCraft",
        options,
        Box::new(|cc| {
            theme::apply(&cc.egui_ctx);
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
        let rect = ui.clip_rect();
        widgets::horizontal_gradient(ui.painter(), rect, theme::HEADER_LEFT, theme::HEADER_RIGHT);
        ui.painter().hline(
            rect.x_range(),
            rect.bottom() - 0.5,
            Stroke::new(1.0, theme::ACCENT.gamma_multiply(0.35)),
        );

        if ui.available_width() < 720.0 {
            self.title_block(ui);
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                self.status(ui);
                self.countdown(ui);
                self.refresh_button(ui);
            });
        } else {
            ui.horizontal(|ui| {
                self.title_block(ui);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.refresh_button(ui);
                    self.countdown(ui);
                    self.status(ui);
                });
            });
        }
    }

    fn title_block(&self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.label(
                RichText::new("☁  METEOCRAFT")
                    .size(12.0)
                    .strong()
                    .color(theme::ACCENT),
            );
            ui.label(
                RichText::new("Ljubljana Koseze")
                    .size(28.0)
                    .strong()
                    .color(theme::TEXT),
            );
            let subtitle = match &self.data {
                Some(d) => format!(
                    "Weather station  ·  last measurement {}",
                    d.last_measurement.replace(" ob ", " at ")
                ),
                None => "Weather station".to_owned(),
            };
            ui.label(muted(subtitle).size(13.5));
        });
    }

    fn status(&self, ui: &mut egui::Ui) {
        if self.pending.is_some() {
            widgets::status_pill(ui, "Updating", theme::ACCENT);
        } else if let Some(err) = &self.error {
            widgets::status_pill(ui, "Offline", theme::BAD).on_hover_text(err);
        } else if self.data.is_some() {
            widgets::status_pill(ui, "Live", theme::GOOD);
        }
    }

    fn countdown(&self, ui: &mut egui::Ui) {
        if self.pending.is_none() {
            let left = REFRESH_INTERVAL.saturating_sub(self.last_fetch.elapsed());
            ui.label(
                muted(format!(
                    "next update in {}:{:02}",
                    left.as_secs() / 60,
                    left.as_secs() % 60
                ))
                .size(12.5),
            );
        }
    }

    fn refresh_button(&mut self, ui: &mut egui::Ui) {
        let button = egui::Button::new(RichText::new("⟳  Refresh").strong());
        if ui.add_enabled(self.pending.is_none(), button).clicked() {
            self.refresh();
        }
    }

    fn footer(ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                muted(
                    "Unofficial data from a private LaCrosse WS2300 station, not part of the ARSO \
                     network. For information only.",
                )
                .size(12.0),
            );
            ui.hyperlink_to(
                RichText::new("vreme.zajcek.org/koseze ↗").size(12.0),
                station::STATION_URL,
            );
        });
    }
}

impl eframe::App for MeteoApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_fetch();
        if self.last_fetch.elapsed() >= REFRESH_INTERVAL {
            self.refresh();
        }
        // Repaint once a second so the countdown stays current.
        ctx.request_repaint_after(Duration::from_secs(1));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("header")
            .frame(
                egui::Frame::new()
                    .fill(theme::HEADER_RIGHT)
                    .inner_margin(Margin::symmetric(28, 18)),
            )
            .show_separator_line(false)
            .show(ui, |ui| self.header(ui));

        egui::Panel::bottom("footer")
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(Margin::symmetric(28, 10)),
            )
            .show_separator_line(false)
            .show(ui, Self::footer);

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BG))
            .show(ui, |ui| {
                let Some(data) = &self.data else {
                    self.placeholder(ui);
                    return;
                };
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        egui::Frame::new()
                            .inner_margin(Margin::symmetric(28, 20))
                            .show(ui, |ui| {
                                if let Some(err) = &self.error {
                                    error_banner(ui, err);
                                }
                                dashboard(ui, data);
                            });
                    });
            });
    }
}

impl MeteoApp {
    fn placeholder(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() / 3.0);
            if self.pending.is_some() {
                ui.spinner();
                ui.label(muted("Loading station data…"));
            } else {
                ui.label(RichText::new("⚠").size(36.0).color(theme::BAD));
                ui.label(
                    RichText::new("Could not load station data")
                        .size(18.0)
                        .color(theme::TEXT),
                );
                if let Some(err) = &self.error {
                    ui.label(muted(err.as_str()));
                }
                ui.add_space(8.0);
                if ui.button("⟳  Try again").clicked() {
                    self.refresh();
                }
            }
        });
    }
}

fn error_banner(ui: &mut egui::Ui, err: &str) {
    egui::Frame::new()
        .fill(theme::BAD.gamma_multiply(0.12))
        .stroke(Stroke::new(1.0, theme::BAD.gamma_multiply(0.5)))
        .corner_radius(10)
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(format!("⚠  Showing the last data received. {err}"))
                    .color(theme::BAD),
            );
        });
    ui.add_space(12.0);
}

fn dashboard(ui: &mut egui::Ui, data: &StationData) {
    widgets::card_rows(
        ui,
        "overview",
        420.0,
        vec![
            Box::new(|ui| now_card(ui, data)),
            Box::new(|ui| wind_card(ui, &data.wind)),
        ],
    );

    let metrics: Vec<CardFn> = vec![
        Box::new(|ui| pressure_tile(ui, data)),
        Box::new(|ui| humidity_tile(ui, data)),
        Box::new(|ui| precipitation_tile(ui, data)),
        Box::new(|ui| snow_tile(ui, data)),
    ];
    widgets::card_rows(ui, "metrics", 210.0, metrics);

    widgets::card_rows(ui, "daily", 0.0, vec![Box::new(|ui| daily_stats(ui, data))]);
    widgets::card_rows(ui, "records", 0.0, vec![Box::new(|ui| records(ui, data))]);

    let images: Vec<CardFn> = vec![
        Box::new(|ui| image_card(ui, "📷", "Live camera · facing SW", "livefeed/live.jpg")),
        Box::new(|ui| image_card(ui, "📈", "Temperature and wind chill", "graf.png")),
        Box::new(|ui| image_card(ui, "📉", "Air pressure", "graf_tlak.png")),
    ];
    widgets::card_rows(ui, "images", 300.0, images);
}

fn now_card(ui: &mut egui::Ui, data: &StationData) {
    card_title(ui, "🌡", "Now");
    let temp_text = value_of(data.reading("temperatura"));
    let temp = number(temp_text);
    let temp_color = temp.map_or(theme::TEXT, theme::temperature_color);

    ui.horizontal(|ui| {
        ui.label(RichText::new(temp_text).size(64.0).color(temp_color));
        ui.add_space(8.0);
        ui.vertical(|ui| {
            ui.add_space(8.0);
            ui.label(RichText::new(conditions(data)).size(17.0).color(theme::TEXT));
            ui.label(muted(format!(
                "Feels like {}",
                value_of(data.reading("občutek mraza"))
            )));
            ui.label(muted(format!("Dew point {}", value_of(data.reading("rosišče")))));
        });
    });

    if let Some(stat) = data.daily_stat("temperatura") {
        let min = stat.min.as_deref().and_then(number);
        let max = stat.max.as_deref().and_then(number);
        if let (Some(min), Some(max)) = (min, max) {
            ui.add_space(10.0);
            caption(ui, "Today");
            ui.add_space(2.0);
            let avg = stat.avg.as_deref().and_then(number);
            widgets::temperature_range(ui, min, max, avg, temp);
        }
    }
}

fn wind_card(ui: &mut egui::Ui, wind: &station::Wind) {
    card_title(ui, "🌬", "Wind");
    let compass_point = wind.direction.split_whitespace().next().unwrap_or("—");
    ui.horizontal(|ui| {
        widgets::compass(ui, wind.direction_deg, 168.0);
        ui.add_space(8.0);
        ui.vertical(|ui| {
            ui.add_space(6.0);
            caption(ui, "Speed");
            match wind.speed_kmh {
                Some(kmh) => {
                    ui.label(RichText::new(format!("{kmh:.1} km/h")).size(28.0).color(theme::TEXT));
                    ui.label(muted(format!("{:.1} m/s  ·  {}", kmh / 3.6, beaufort(kmh))));
                }
                None => {
                    ui.label(RichText::new(&wind.speed).size(20.0).color(theme::TEXT));
                }
            }
            ui.add_space(10.0);
            caption(ui, "Direction");
            ui.label(RichText::new(compass_point).size(28.0).color(theme::TEXT));
            let mut detail = format!("from the {}", direction_name(compass_point));
            if let Some(deg) = wind.direction_deg {
                detail = format!("{deg}°  ·  {detail}");
            }
            ui.label(muted(detail));
        });
    });
}

fn pressure_tile(ui: &mut egui::Ui, data: &StationData) {
    let detail = data
        .daily_stat("tlak")
        .and_then(|s| Some(format!("today {} – {}", s.min.as_deref()?, s.max.as_deref()?)))
        .unwrap_or_default();
    widgets::metric(
        ui,
        "⏲",
        "Air pressure",
        theme::ACCENT,
        value_of(data.reading("zračni tlak")),
        &detail,
    );
}

fn humidity_tile(ui: &mut egui::Ui, data: &StationData) {
    let value = value_of(data.reading("vlažnost"));
    let detail = data
        .daily_stat("vlaga")
        .and_then(|s| Some(format!("today {} – {}", s.min.as_deref()?, s.max.as_deref()?)))
        .unwrap_or_default();
    widgets::metric(ui, "💧", "Humidity", theme::HUMID, value, &detail);
    if let Some(h) = number(value) {
        ui.add_space(4.0);
        ui.add(
            egui::ProgressBar::new(h / 100.0)
                .desired_height(6.0)
                .fill(theme::HUMID)
                .corner_radius(3),
        );
    }
}

fn precipitation_tile(ui: &mut egui::Ui, data: &StationData) {
    widgets::metric(
        ui,
        "☔",
        "Rain, last hour",
        theme::COLD,
        value_of(data.precipitation("padavine v zadnji uri")),
        &format!("last 24 h: {}", value_of(data.precipitation("padavine zadnjih 24h"))),
    );
}

fn snow_tile(ui: &mut egui::Ui, data: &StationData) {
    let value = value_of(data.precipitation("višina snežne odeje"));
    let detail = if number(value).is_some() {
        "snow cover"
    } else {
        "no snow reported"
    };
    widgets::metric(ui, "❄", "Snow depth", theme::SNOW, value, detail);
}

fn daily_stats(ui: &mut egui::Ui, data: &StationData) {
    card_title(ui, "📊", "Today's statistics");
    let col_width = (ui.available_width() / 4.0 - 16.0).max(64.0);
    egui::Grid::new("daily")
        .num_columns(4)
        .striped(true)
        .spacing([16.0, 8.0])
        .min_col_width(col_width)
        .show(ui, |ui| {
            ui.label("");
            for title in ["minimum", "maximum", "average"] {
                caption(ui, title);
            }
            ui.end_row();
            for stat in &data.daily {
                ui.label(RichText::new(english(&stat.label)).color(theme::TEXT));
                for (value, color) in [
                    (&stat.min, theme::COLD),
                    (&stat.max, theme::WARM),
                    (&stat.avg, theme::TEXT),
                ] {
                    match value {
                        Some(v) => ui.label(RichText::new(v).color(color)),
                        None => ui.label(muted("—")),
                    };
                }
                ui.end_row();
            }
        });
}

fn records(ui: &mut egui::Ui, data: &StationData) {
    card_title(ui, "🏆", "All-time records");
    let col_width = (ui.available_width() / 3.0 - 16.0).max(90.0);
    let date_inline = ui.available_width() > 760.0;
    egui::Grid::new("records")
        .num_columns(3)
        .striped(true)
        .spacing([16.0, 8.0])
        .min_col_width(col_width)
        .show(ui, |ui| {
            ui.label("");
            caption(ui, "minimum");
            caption(ui, "maximum");
            ui.end_row();
            for record in &data.records {
                ui.label(RichText::new(english(&record.label)).color(theme::TEXT));
                for (extreme, color) in [(&record.min, theme::COLD), (&record.max, theme::WARM)] {
                    match extreme {
                        Some(e) => {
                            let show = |ui: &mut egui::Ui| {
                                ui.label(RichText::new(&e.value).color(color).strong());
                                ui.label(muted(&e.when).size(12.0));
                            };
                            if date_inline {
                                ui.horizontal(show);
                            } else {
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 0.0;
                                    show(ui);
                                });
                            }
                        }
                        None => {
                            ui.label(muted("—"));
                        }
                    }
                }
                ui.end_row();
            }
        });
}

fn image_card(ui: &mut egui::Ui, icon: &str, title: &str, path: &str) {
    card_title(ui, icon, title);
    let url = station::image_url(path);
    // Inside a scroll area the available height is not meaningful, so fit by width
    // (all station images are wider than they are tall).
    let image = egui::Image::new(&url)
        .fit_to_exact_size(Vec2::splat(ui.available_width()))
        .corner_radius(10)
        .show_loading_spinner(true);
    let response = ui.add(image.sense(Sense::click()));
    if response.on_hover_text("Open in browser").clicked() {
        ui.ctx().open_url(egui::OpenUrl::new_tab(url));
    }
}

fn value_of(reading: Option<&station::Reading>) -> &str {
    reading.map_or("—", |r| r.value.as_str())
}

/// The leading number of a value such as "8.6 °C" or "86%".
fn number(value: &str) -> Option<f32> {
    value
        .split_whitespace()
        .next()?
        .trim_end_matches('%')
        .parse()
        .ok()
}

/// A short plain-English summary, e.g. "Cool and humid".
fn conditions(data: &StationData) -> String {
    let temp = number(value_of(data.reading("temperatura")));
    let feel = match temp {
        Some(t) if t < 0.0 => "Freezing",
        Some(t) if t < 8.0 => "Cold",
        Some(t) if t < 15.0 => "Cool",
        Some(t) if t < 22.0 => "Mild",
        Some(t) if t < 28.0 => "Warm",
        Some(_) => "Hot",
        None => "No reading",
    };
    let raining = number(value_of(data.precipitation("padavine v zadnji uri"))).is_some_and(|mm| mm > 0.0);
    let humid = number(value_of(data.reading("vlažnost"))).is_some_and(|h| h >= 85.0);
    match (raining, humid) {
        (true, _) => format!("{feel}, rain falling"),
        (false, true) => format!("{feel} and humid"),
        (false, false) => feel.to_owned(),
    }
}

fn beaufort(kmh: f32) -> &'static str {
    const SCALE: [(f32, &str); 12] = [
        (1.0, "calm"),
        (6.0, "light air"),
        (12.0, "light breeze"),
        (20.0, "gentle breeze"),
        (29.0, "moderate breeze"),
        (39.0, "fresh breeze"),
        (50.0, "strong breeze"),
        (62.0, "near gale"),
        (75.0, "gale"),
        (89.0, "strong gale"),
        (103.0, "storm"),
        (118.0, "violent storm"),
    ];
    SCALE
        .iter()
        .find(|(limit, _)| kmh < *limit)
        .map_or("hurricane", |(_, name)| name)
}

fn direction_name(point: &str) -> &str {
    match point {
        "N" => "north",
        "NNE" => "north-northeast",
        "NE" => "northeast",
        "ENE" => "east-northeast",
        "E" => "east",
        "ESE" => "east-southeast",
        "SE" => "southeast",
        "SSE" => "south-southeast",
        "S" => "south",
        "SSW" => "south-southwest",
        "SW" => "southwest",
        "WSW" => "west-southwest",
        "W" => "west",
        "WNW" => "west-northwest",
        "NW" => "northwest",
        "NNW" => "north-northwest",
        other => other,
    }
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
        "najzgodnejše sneženje in snežna odeja" => "earliest snowfall",
        "najpoznejše sneženje in snežne odeja" => "latest snowfall",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_label_on_the_page_has_an_english_name() {
        let data = station::parse(include_str!("../tests/fixtures/koseze.html")).unwrap();
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

    #[test]
    fn extracts_leading_numbers() {
        assert_eq!(number("8.6 °C"), Some(8.6));
        assert_eq!(number("86%"), Some(86.0));
        assert_eq!(number("—"), None);
    }

    #[test]
    fn summarises_conditions() {
        let data = station::parse(include_str!("../tests/fixtures/koseze.html")).unwrap();
        assert_eq!(conditions(&data), "Cool and humid");
        assert_eq!(beaufort(0.0), "calm");
        assert_eq!(beaufort(25.0), "moderate breeze");
    }

    #[test]
    fn temperature_colors_are_continuous() {
        let a = theme::temperature_color(9.99);
        let b = theme::temperature_color(10.01);
        assert!(a.r().abs_diff(b.r()) < 3 && a.g().abs_diff(b.g()) < 3);
    }
}
