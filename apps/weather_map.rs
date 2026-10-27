//! Clouds seen by Meteosat and rain seen by weather radars around Slovenia over the last
//! hour: the newest picture on a card, and the hour as a loop in a window of its own.

mod download;

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Color32, Pos2, Rect, RichText, Sense, TextureFilter, TextureHandle, TextureOptions, Vec2,
};

use crate::theme;
use crate::widgets::{card_title, map_marker, muted};
use download::{Frame, Update};

/// Both sources publish a picture every 10 minutes.
const PERIOD: Duration = Duration::from_secs(600);
/// After a failed download, before trying the missing pictures again.
const RETRY: Duration = Duration::from_secs(60);
/// The last hour, 10 minutes apart.
const FRAMES: usize = 7;
const FRAME_TIME: Duration = Duration::from_millis(500);
/// The newest picture stays a little longer before the loop starts over.
const LAST_FRAME_TIME: Duration = Duration::from_millis(1500);
const ATTRIBUTION: &str =
    "Clouds © EUMETSAT · radar RainViewer · map © OpenStreetMap contributors";
/// RainViewer's colors for rain, from drizzle to hail, by radar reflectivity in dBZ.
const RAIN_SCALE: [(u8, Color32); 12] = [
    (10, Color32::from_rgb(0xce, 0xc0, 0x87)),
    (15, Color32::from_rgb(0x88, 0xdd, 0xee)),
    (20, Color32::from_rgb(0x00, 0xa3, 0xe0)),
    (25, Color32::from_rgb(0x00, 0x77, 0xaa)),
    (30, Color32::from_rgb(0x00, 0x55, 0x88)),
    (35, Color32::from_rgb(0xff, 0xee, 0x00)),
    (40, Color32::from_rgb(0xff, 0xaa, 0x00)),
    (45, Color32::from_rgb(0xff, 0x44, 0x00)),
    (50, Color32::from_rgb(0xc1, 0x00, 0x00)),
    (55, Color32::from_rgb(0xff, 0xaa, 0xff)),
    (60, Color32::from_rgb(0xff, 0x77, 0xff)),
    (65, Color32::WHITE),
];
/// The pictures are drawn much smaller than they are, especially on the card.
const SMOOTH: TextureOptions = TextureOptions::LINEAR.with_mipmap_mode(Some(TextureFilter::Linear));

pub struct WeatherMap {
    map: Option<TextureHandle>,
    /// The frames of the loop, oldest first.
    frames: Vec<Frame>,
    radar: HashMap<i64, TextureHandle>,
    clouds: HashMap<i64, TextureHandle>,
    pending: Option<Receiver<Update>>,
    fetched: Option<Instant>,
    problem: Option<String>,
    show_clouds: bool,
    show_radar: bool,
    playing: bool,
    /// The frame shown in the window, among `shown()`, and since when.
    index: usize,
    since: Instant,
}

impl WeatherMap {
    pub fn new() -> Self {
        Self {
            map: None,
            frames: Vec::new(),
            radar: HashMap::new(),
            clouds: HashMap::new(),
            pending: None,
            fetched: None,
            problem: None,
            show_clouds: true,
            show_radar: true,
            playing: true,
            index: 0,
            since: Instant::now(),
        }
    }

    /// Set for a frame by a click on the card.
    pub fn request_id() -> egui::Id {
        egui::Id::unique("open-weather-map")
    }

    /// Starts the loop over from its oldest picture.
    pub fn rewind(&mut self) {
        self.playing = true;
        self.index = 0;
        self.since = Instant::now();
    }

    /// Downloads what's new, when due. Called only while the pictures are on screen.
    fn refresh_if_due(&mut self, ctx: &egui::Context) {
        let wait = if self.problem.is_some() { RETRY } else { PERIOD };
        let due = self.fetched.is_none_or(|at| at.elapsed() >= wait);
        if self.pending.is_some() || !due {
            return;
        }
        let have_map = self.map.is_some();
        let have_radar: Vec<i64> = self.radar.keys().copied().collect();
        let have_clouds: Vec<i64> = self.clouds.keys().copied().collect();
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(download::fetch(FRAMES, have_map, &have_radar, &have_clouds));
            ctx.request_repaint();
        });
        self.pending = Some(rx);
        self.fetched = Some(Instant::now());
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.pending else { return };
        let update = match rx.try_recv() {
            Ok(update) => update,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.pending = None;
                self.problem = Some("download thread stopped unexpectedly".into());
                return;
            }
        };
        self.pending = None;
        self.problem = update.problem;
        if let Some(map) = update.map {
            self.map = Some(ctx.load_texture("weather-map", map, SMOOTH));
        }
        for (time, image) in update.radar {
            let texture = ctx.load_texture(format!("radar-{time}"), image, SMOOTH);
            self.radar.insert(time, texture);
        }
        for (time, image) in update.clouds {
            let texture = ctx.load_texture(format!("clouds-{time}"), image, SMOOTH);
            self.clouds.insert(time, texture);
        }
        // Without a new list of pictures, keep showing the last one.
        if !update.frames.is_empty() {
            self.frames = update.frames;
            let frames = &self.frames;
            self.radar.retain(|time, _| frames.iter().any(|f| f.radar == *time));
            self.clouds.retain(|time, _| frames.iter().any(|f| f.clouds == *time));
        }
    }

    /// The frames that can be shown: those with radar, or with clouds if there is no radar.
    fn shown(&self) -> Vec<Frame> {
        let with_radar: Vec<Frame> = (self.frames.iter())
            .filter(|f| self.radar.contains_key(&f.radar))
            .copied()
            .collect();
        if !with_radar.is_empty() {
            return with_radar;
        }
        (self.frames.iter())
            .filter(|f| self.clouds.contains_key(&f.clouds))
            .copied()
            .collect()
    }

    fn describe(&self, frame: Frame) -> String {
        let mut parts = Vec::new();
        if self.radar.contains_key(&frame.radar) {
            parts.push(format!("radar {}", download::ljubljana_clock(frame.radar)));
        }
        if self.clouds.contains_key(&frame.clouds) {
            parts.push(format!("clouds {}", download::ljubljana_clock(frame.clouds)));
        }
        parts.join(" · ")
    }

    /// Draws one frame's layers over the map, and Koseze, filling `rect`.
    fn paint(&self, ui: &egui::Ui, rect: Rect, frame: Option<Frame>, corner_radius: u8) {
        let layer = |texture: &TextureHandle| {
            egui::Image::new((texture.id(), rect.size()))
                .corner_radius(corner_radius)
                .paint_at(ui, rect);
        };
        match &self.map {
            Some(map) => layer(map),
            None => {
                ui.painter().rect_filled(rect, corner_radius, theme::TRACK);
            }
        }
        if let Some(frame) = frame {
            if self.show_clouds
                && let Some(clouds) = self.clouds.get(&frame.clouds)
            {
                layer(clouds);
            }
            if self.show_radar
                && let Some(radar) = self.radar.get(&frame.radar)
            {
                layer(radar);
            }
        }
        let [x, y] = download::koseze();
        map_marker(ui.painter(), rect.lerp_inside(Vec2::new(x, y)));
    }

    /// The newest picture, which opens the loop when clicked.
    pub fn card(&mut self, ui: &mut egui::Ui) {
        self.refresh_if_due(ui.ctx());
        card_title(ui, "☁", "Clouds and radar");
        let width = ui.available_width();
        let size = Vec2::new(width, width * download::SIZE[1] as f32 / download::SIZE[0] as f32);
        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
        let latest = self.shown().last().copied();
        self.paint(ui, rect, latest, 10);
        if self.map.is_none() && latest.is_none() && self.pending.is_some() {
            ui.put(rect, egui::Spinner::new());
        }
        if response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("Play the last hour")
            .clicked()
        {
            ui.ctx().data_mut(|d| d.insert_temp(Self::request_id(), true));
            ui.ctx().request_repaint();
        }
        let status = match (latest, &self.problem) {
            (Some(frame), _) => self.describe(frame),
            (None, Some(problem)) => format!("⚠ {problem}"),
            (None, None) => "Loading the satellite and radar pictures…".to_owned(),
        };
        ui.label(muted(status).size(12.5));
        ui.label(muted(ATTRIBUTION).size(11.0));
    }

    /// The last hour as a loop, with controls.
    pub fn window(&mut self, ui: &mut egui::Ui) {
        self.refresh_if_due(ui.ctx());
        let frames = self.shown();
        self.index = self.index.min(frames.len().saturating_sub(1));
        self.advance(ui.ctx(), frames.len());
        let frame = frames.get(self.index).copied();

        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Clouds and radar")
                    .size(24.0)
                    .strong()
                    .color(theme::TEXT),
            );
            if self.pending.is_some() {
                ui.spinner();
            }
        });
        ui.label(muted(
            "Slovenia and around it in the last hour · Meteosat clouds and weather radar rain · \
             times in Ljubljana",
        ));
        if let Some(problem) = &self.problem {
            ui.label(RichText::new(format!("⚠  Some pictures are missing: {problem}")).color(theme::BAD));
        }
        ui.add_space(8.0);

        // Room below the map for the controls, the legend and the credits.
        const BELOW: f32 = 96.0;
        let free = ui.available_size() - Vec2::new(0.0, BELOW);
        let aspect = download::SIZE[0] as f32 / download::SIZE[1] as f32;
        let width = free.x.min(free.y * aspect).max(120.0);
        let size = Vec2::new(width, width / aspect);
        ui.vertical_centered(|ui| {
            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
            self.paint(ui, rect, frame, 12);
            if let Some(frame) = frame {
                time_badge(ui, rect, &download::ljubljana_clock(frame.radar));
            } else if self.pending.is_some() {
                ui.put(rect, egui::Spinner::new().size(32.0));
            }
        });
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            let label = if self.playing { "⏸  Pause" } else { "▶  Play" };
            let button = egui::Button::new(RichText::new(label).strong());
            if ui.add_enabled(frames.len() > 1, button).clicked() {
                self.playing = !self.playing;
                self.since = Instant::now();
            }
            if let Some(last) = frames.len().checked_sub(1) {
                ui.spacing_mut().slider_width = (ui.available_width() - 420.0).clamp(120.0, 520.0);
                let slider = egui::Slider::new(&mut self.index, 0..=last).show_value(false);
                if ui.add_enabled(last > 0, slider).changed() {
                    self.playing = false;
                }
            }
            if let Some(frame) = frame {
                ui.label(RichText::new(self.describe(frame)).color(theme::TEXT));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.checkbox(&mut self.show_radar, "Rain");
                ui.checkbox(&mut self.show_clouds, "Clouds");
            });
        });
        ui.add_space(4.0);
        rain_legend(ui);
        ui.add_space(2.0);
        ui.label(muted(ATTRIBUTION).size(11.0));
    }

    fn advance(&mut self, ctx: &egui::Context, count: usize) {
        if !self.playing || count < 2 {
            return;
        }
        let hold = |index: usize| {
            if index + 1 >= count {
                LAST_FRAME_TIME
            } else {
                FRAME_TIME
            }
        };
        let elapsed = self.since.elapsed();
        if elapsed >= hold(self.index) {
            self.index = (self.index + 1) % count;
            self.since = Instant::now();
            ctx.request_repaint_after(hold(self.index));
        } else {
            ctx.request_repaint_after(hold(self.index) - elapsed);
        }
    }
}

/// The time of the picture in a corner of the map.
fn time_badge(ui: &egui::Ui, map: Rect, text: &str) {
    let painter = ui.painter();
    let galley = painter.layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(20.0),
        theme::TEXT,
    );
    let at = map.left_top() + Vec2::new(20.0, 14.0);
    let badge = Rect::from_min_size(at, galley.size()).expand2(Vec2::new(10.0, 4.0));
    painter.rect_filled(badge, 8, theme::BG.gamma_multiply(0.8));
    painter.galley(at, galley, theme::TEXT);
}

fn rain_legend(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.label(muted("Rain:  light").size(12.0));
        const SWATCH: Vec2 = Vec2::new(22.0, 10.0);
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(SWATCH.x * RAIN_SCALE.len() as f32, SWATCH.y),
            Sense::hover(),
        );
        for (i, (_, color)) in RAIN_SCALE.iter().enumerate() {
            let swatch = Rect::from_min_size(
                Pos2::new(rect.left() + SWATCH.x * i as f32, rect.top()),
                SWATCH,
            );
            ui.painter().rect_filled(swatch, 0, *color);
        }
        response.on_hover_text(
            "Radar reflectivity from 10 to 65 dBZ: drizzle, rain, heavy rain, then hail. \
             Snow is drawn from white to blue.",
        );
        ui.label(muted("heavy, hail").size(12.0));
    });
}
