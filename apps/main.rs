#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod alerts;
mod app_window;
mod charts;
mod history;
mod map;
#[cfg(windows)]
mod notify;
mod sky;
mod station;
mod theme;
#[cfg(windows)]
mod tray;
mod widgets;

use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, FontId, Margin, Pos2, Rect, RichText, Sense, Stroke, Vec2, ViewportCommand,
};

use sky::Sky;
use station::{FetchError, StationData};
use widgets::{CardFn, caption, card_title, muted};

/// The station page itself refreshes every 5 minutes.
const REFRESH_INTERVAL: Duration = Duration::from_secs(300);
/// Until the first download succeeds (e.g. the network isn't up yet right after sign-in).
const RETRY_INTERVAL: Duration = Duration::from_secs(30);
/// The weather symbol in the header, in points, and the resolution it is drawn at.
const SKY_ICON_SIZE: f32 = 54.0;
const SKY_ICON_PIXELS: usize = 160;
/// Keys of the settings eframe saves between runs.
const ALERTS_KEY: &str = "alerts";
const CHARTS_PLACEMENT_KEY: &str = "charts-window";

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("MeteoCraft – Ljubljana Koseze")
            .with_inner_size([1000.0, 750.0])
            .with_min_inner_size([460.0, 440.0]),
        wgpu_options: wgpu_options(),
        ..Default::default()
    };
    eframe::run_native(
        "MeteoCraft",
        options,
        Box::new(|cc| {
            theme::apply(&cc.egui_ctx);
            egui_extras::install_image_loaders(&cc.egui_ctx);
            Ok(Box::new(MeteoApp::new(cc.egui_ctx.clone(), cc.storage)))
        }),
    )
}

/// On Windows wgpu would otherwise pick Vulkan on NVIDIA cards, whose driver kept
/// tens of GB of GPU memory committed to the process after a few hours.
/// `WGPU_BACKEND` still overrides this.
#[cfg(windows)]
fn wgpu_options() -> eframe::egui_wgpu::WgpuConfiguration {
    use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew, wgpu};
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends =
        wgpu::Backends::from_env().unwrap_or(wgpu::Backends::DX12);
    WgpuConfiguration {
        wgpu_setup: WgpuSetup::CreateNew(setup),
        ..Default::default()
    }
}

#[cfg(not(windows))]
fn wgpu_options() -> eframe::egui_wgpu::WgpuConfiguration {
    Default::default()
}

struct MeteoApp {
    ctx: egui::Context,
    data: Option<StationData>,
    error: Option<String>,
    pending: Option<Receiver<Result<StationData, FetchError>>>,
    last_fetch: Instant,
    map: Map,
    sky: Option<(Sky, egui::TextureHandle)>,
    sky_pending: Option<Receiver<Result<Sky, String>>>,
    /// When the last successful sky observation was requested.
    sky_fetched: Option<Instant>,
    #[cfg(windows)]
    tray: Option<tray::Tray>,
    /// Set when the user picks Quit, so closing the window really exits.
    quitting: bool,
    /// Launched at sign-in: hide the window on the first frame and live in the tray.
    start_in_tray: bool,
    /// The window with the last 24 hours as charts, while it is open.
    charts: Option<ChartsWindow>,
    /// Where the charts window was last, to open it there again.
    charts_placement: Option<Placement>,
    alerts: alerts::Watch,
    /// Sent when a notification is clicked, from whichever thread Windows reports it on.
    alert_clicks: (Sender<()>, Receiver<()>),
}

/// A window's position and inner size, in points.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Placement {
    position: Pos2,
    size: Vec2,
}

impl Placement {
    fn parse(text: &str) -> Option<Self> {
        let values: Vec<f32> = text
            .split(',')
            .map(|v| v.trim().parse().ok())
            .collect::<Option<_>>()?;
        let [x, y, width, height] = values[..] else {
            return None;
        };
        Some(Self {
            position: Pos2::new(x, y),
            size: Vec2::new(width, height),
        })
    }

    fn to_text(self) -> String {
        format!(
            "{},{},{},{}",
            self.position.x, self.position.y, self.size.x, self.size.y
        )
    }

    /// Whether the top of the window, where it can be dragged, is on a monitor. A saved
    /// position may be on a monitor that has since been disconnected.
    #[cfg(windows)]
    fn is_on_screen(&self, pixels_per_point: f32) -> bool {
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONULL, MonitorFromPoint};
        let point = POINT {
            x: ((self.position.x + self.size.x / 2.0) * pixels_per_point) as i32,
            y: ((self.position.y + 10.0) * pixels_per_point) as i32,
        };
        !unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONULL) }.is_null()
    }

    #[cfg(not(windows))]
    fn is_on_screen(&self, _pixels_per_point: f32) -> bool {
        true
    }
}

struct ChartsWindow {
    history: Option<history::History>,
    error: Option<String>,
    pending: Option<Receiver<Result<history::History, String>>>,
    size: egui::Vec2,
    position: Option<egui::Pos2>,
}

impl ChartsWindow {
    /// Tall enough to show all the charts without scrolling.
    const SIZE: egui::Vec2 = egui::vec2(1060.0, 1060.0);
    /// Leaves room for the title bar and the taskbar.
    const SCREEN_MARGIN: f32 = 140.0;

    /// Opens where the window was last if that is still on screen, otherwise centered.
    fn new(last: Option<Placement>, monitor: Option<egui::Vec2>, pixels_per_point: f32) -> Self {
        let (size, position) = match last {
            Some(last) if last.is_on_screen(pixels_per_point) => (last.size, Some(last.position)),
            _ => {
                let size = match monitor {
                    Some(monitor) => {
                        Self::SIZE.min(monitor - egui::Vec2::splat(Self::SCREEN_MARGIN))
                    }
                    None => Self::SIZE,
                };
                let centered =
                    monitor.map(|monitor| ((monitor - size) / 2.0).max(egui::Vec2::ZERO).to_pos2());
                (size, centered)
            }
        };
        Self {
            history: None,
            error: None,
            pending: None,
            size,
            position,
        }
    }

    fn viewport_id() -> egui::ViewportId {
        egui::ViewportId::from_hash_of("charts")
    }

    /// Set for a frame by a click on one of the station's graphs.
    fn request_id() -> egui::Id {
        egui::Id::unique("open-charts")
    }

    fn load(&mut self, ctx: &egui::Context) {
        if self.pending.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(history::fetch());
            ctx.request_repaint();
        });
        self.pending = Some(rx);
    }

    fn poll(&mut self) {
        let Some(rx) = &self.pending else { return };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("download thread stopped unexpectedly".into()),
        };
        self.pending = None;
        match result {
            Ok(history) => {
                self.history = Some(history);
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
    }

    fn ui(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Last 24 hours")
                    .size(24.0)
                    .strong()
                    .color(theme::TEXT),
            );
            if self.pending.is_some() {
                ui.spinner();
            }
        });
        let until = self
            .history
            .as_ref()
            .and_then(|h| h.temperature.last())
            .map_or_else(String::new, |p| {
                format!(" · up to {}", charts::clock(p.minutes))
            });
        ui.label(muted(format!(
            "Ljubljana Koseze, read from the station's graphs{until}"
        )));
        if let Some(err) = &self.error {
            ui.label(
                RichText::new(format!("⚠  Could not read the graphs: {err}")).color(theme::BAD),
            );
        }
        ui.add_space(10.0);
        match &self.history {
            Some(history) => {
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| charts::show(ui, history));
            }
            None if self.pending.is_some() => {
                ui.label(muted("Reading the station's graphs…"));
            }
            None => {}
        }
    }
}

enum Map {
    Loading(Receiver<Result<map::Map, String>>),
    /// The map and its attribution.
    Ready(egui::TextureHandle, String),
    Failed(String),
}

impl Map {
    fn load(ctx: &egui::Context) -> Self {
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(map::load());
            ctx.request_repaint();
        });
        Map::Loading(rx)
    }
}

impl MeteoApp {
    fn new(ctx: egui::Context, storage: Option<&dyn eframe::Storage>) -> Self {
        let saved = |key| storage.and_then(|s| s.get_string(key));
        let mut app = Self {
            map: Map::load(&ctx),
            #[cfg(windows)]
            tray: tray::Tray::new(&ctx)
                .inspect_err(|e| eprintln!("tray icon unavailable: {e}"))
                .ok(),
            ctx,
            data: None,
            error: None,
            pending: None,
            last_fetch: Instant::now(),
            sky: None,
            sky_pending: None,
            sky_fetched: None,
            quitting: false,
            #[cfg(windows)]
            start_in_tray: std::env::args().any(|arg| arg == tray::TRAY_ARG),
            #[cfg(not(windows))]
            start_in_tray: false,
            charts: None,
            charts_placement: saved(CHARTS_PLACEMENT_KEY)
                .as_deref()
                .and_then(Placement::parse),
            alerts: alerts::Watch::new(saved(ALERTS_KEY).as_deref()),
            alert_clicks: mpsc::channel(),
        };
        app.refresh();
        app
    }

    fn has_tray(&self) -> bool {
        #[cfg(windows)]
        return self.tray.is_some();
        #[cfg(not(windows))]
        false
    }

    #[cfg(windows)]
    fn handle_tray(&mut self, ctx: &egui::Context) {
        while let Some(command) = self.tray.as_ref().and_then(tray::Tray::poll) {
            match command {
                tray::Command::Show => show_window(ctx),
                tray::Command::Refresh => self.refresh(),
                tray::Command::Quit => {
                    self.quitting = true;
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            }
        }
    }

    fn update_tray(&self) {
        #[cfg(windows)]
        if let Some(tray) = &self.tray {
            let temp = self
                .data
                .as_ref()
                .and_then(|d| number(value_of(d.reading("temperatura"))));
            let sky = self.sky.as_ref().map(|(sky, _)| sky);
            let mut tooltip = self
                .data
                .as_ref()
                .map_or_else(|| "MeteoCraft".to_owned(), |d| tray_summary(d, sky));
            if self.error.is_some() {
                tooltip.push_str("\nOffline – showing the last data received");
            }
            tray.update(temp, sky, &tooltip);
        }
    }

    fn refresh_interval(&self) -> Duration {
        if self.data.is_none() {
            RETRY_INTERVAL
        } else {
            REFRESH_INTERVAL
        }
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

        let sky_due = self
            .sky_fetched
            .is_none_or(|at| at.elapsed() >= sky::PICKUP_PERIOD);
        if self.sky_pending.is_none() && sky_due {
            let (tx, rx) = mpsc::channel();
            let ctx = self.ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(sky::fetch());
                ctx.request_repaint();
            });
            self.sky_pending = Some(rx);
            self.sky_fetched = Some(Instant::now());
        }
    }

    fn poll_sky(&mut self) {
        let Some(rx) = &self.sky_pending else { return };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("download thread stopped unexpectedly".into()),
        };
        self.sky_pending = None;
        match result {
            Ok(sky) => {
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [SKY_ICON_PIXELS; 2],
                    &sky::icon::render(&sky, SKY_ICON_PIXELS, 3.0),
                );
                let texture =
                    self.ctx
                        .load_texture("sky-icon", image, egui::TextureOptions::LINEAR);
                self.sky = Some((sky, texture));
                self.update_tray();
            }
            Err(e) => {
                // Keep the last observation and try again with the next refresh.
                eprintln!("sky observation unavailable: {e}");
                self.sky_fetched = None;
            }
        }
    }

    fn poll_fetch(&mut self) {
        let Some(rx) = &self.pending else { return };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(FetchError::Http(
                "download thread stopped unexpectedly".into(),
            )),
        };
        self.pending = None;
        match result {
            Ok(data) => {
                let conditions = alerts::Conditions::of(&data);
                for alert in self.alerts.check(Instant::now(), &conditions) {
                    self.notify(&alert.title, &alert.body);
                }
                self.data = Some(data);
                self.error = None;
                // The graphs are regenerated on the server alongside the data.
                self.ctx.forget_all_images();
                if let Some(charts) = &mut self.charts {
                    charts.load(&self.ctx);
                }
                if matches!(self.map, Map::Failed(_)) {
                    self.map = Map::load(&self.ctx);
                }
            }
            Err(e) => self.error = Some(e.to_string()),
        }
        self.update_tray();
    }

    fn poll_map(&mut self) {
        let Map::Loading(rx) = &self.map else { return };
        self.map = match rx.try_recv() {
            Ok(Ok(map)) => Map::Ready(
                self.ctx
                    .load_texture("koseze-map", map.image, egui::TextureOptions::LINEAR),
                map.attribution,
            ),
            Ok(Err(e)) => Map::Failed(e),
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Map::Failed("map thread stopped unexpectedly".into())
            }
        };
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
            ui.horizontal(|ui| {
                let title = self.title_block(ui);
                self.sky_badge(ui, rect.center().x, title.center().y);
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                self.status(ui);
                self.countdown(ui);
                self.refresh_button(ui);
                self.alerts_button(ui);
            });
        } else {
            ui.horizontal(|ui| {
                let title = self.title_block(ui);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.refresh_button(ui);
                    self.alerts_button(ui);
                    self.countdown(ui);
                    self.status(ui);
                    self.sky_badge(ui, rect.center().x, title.center().y);
                });
            });
        }
    }

    /// The weather symbol and its description, centered on `center_x` as far as the
    /// space left between the header's other parts allows.
    fn sky_badge(&self, ui: &mut egui::Ui, center_x: f32, center_y: f32) {
        let Some((sky, texture)) = &self.sky else {
            return;
        };
        const GAP: f32 = 10.0;
        const MARGIN: f32 = 16.0;
        let painter = ui.painter();
        let title = painter.layout_no_wrap(
            sky.description.clone(),
            FontId::proportional(19.0),
            theme::TEXT,
        );
        let source = painter.layout_no_wrap(
            format!("{} · {}", sky::SOURCE, sky.observed),
            FontId::proportional(12.0),
            theme::MUTED,
        );
        let text_width = title.size().x.max(source.size().x);

        let free = ui
            .available_rect_before_wrap()
            .shrink2(Vec2::new(MARGIN, 0.0));
        let with_text = SKY_ICON_SIZE + GAP + text_width;
        let width = if free.width() >= with_text {
            with_text
        } else if free.width() >= SKY_ICON_SIZE {
            SKY_ICON_SIZE
        } else {
            return;
        };
        let x = center_x.clamp(free.left() + width / 2.0, free.right() - width / 2.0);
        let badge = Rect::from_center_size(Pos2::new(x, center_y), Vec2::new(width, SKY_ICON_SIZE));

        let icon = Rect::from_min_size(badge.min, Vec2::splat(SKY_ICON_SIZE));
        egui::Image::new((texture.id(), icon.size())).paint_at(ui, icon);
        if width == with_text {
            let top = badge.center().y - (title.size().y + source.size().y) / 2.0;
            let left = icon.right() + GAP;
            let painter = ui.painter();
            painter.galley(Pos2::new(left, top), title.clone(), theme::TEXT);
            painter.galley(Pos2::new(left, top + title.size().y), source, theme::MUTED);
        }
        let response = ui
            .interact(badge, ui.scope_id().with("sky-badge"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(format!(
                "{} – observed by ARSO at Ljubljana Bežigrad, about 3 km east of Koseze, at {}\n\
                 Click to open ARSO's forecast for Ljubljana",
                sky.description, sky.observed
            ));
        if response.clicked() {
            open_in_window(ui.ctx(), sky::FORECAST_URL, self.has_tray());
        }
    }

    fn title_block(&self, ui: &mut egui::Ui) -> Rect {
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
        })
        .response
        .rect
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
            let left = self
                .refresh_interval()
                .saturating_sub(self.last_fetch.elapsed());
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

    /// Which alerts to send, in a menu that stays open while they are ticked.
    fn alerts_button(&mut self, ui: &mut egui::Ui) {
        let any = alerts::Kind::ALL
            .into_iter()
            .any(|kind| self.alerts.is_enabled(kind));
        let label = if any {
            "🔔  Alerts"
        } else {
            "🔕  Alerts off"
        };
        let config = egui::containers::menu::MenuConfig::new()
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
        let response = egui::containers::menu::MenuButton::new(RichText::new(label).strong())
            .config(config)
            .ui(ui, |ui| {
                ui.set_width(300.0);
                ui.label(muted("Notify me when…"));
                ui.add_space(4.0);
                for kind in alerts::Kind::ALL {
                    let mut enabled = self.alerts.is_enabled(kind);
                    if ui.checkbox(&mut enabled, kind.name()).changed() {
                        self.alerts.set_enabled(kind, enabled);
                    }
                    ui.indent(kind.name(), |ui| ui.label(muted(kind.rule()).size(12.0)));
                    ui.add_space(2.0);
                }
                ui.separator();
                if ui.button("Send a test notification").clicked() {
                    self.notify(
                        "🔔 MeteoCraft alerts",
                        "Alerts about the weather at Koseze will look like this.",
                    );
                }
            });
        response.0.on_hover_text(
            "Windows notifications for frost, wind, rain and falling pressure at Koseze",
        );
    }

    fn notify(&self, title: &str, body: &str) {
        #[cfg(windows)]
        {
            let (clicks, ctx) = (self.alert_clicks.0.clone(), self.ctx.clone());
            notify::show(title, body, move || {
                let _ = clicks.send(());
                ctx.request_repaint();
            });
        }
        #[cfg(not(windows))]
        let _ = (title, body);
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
        #[cfg(windows)]
        self.handle_tray(ctx);
        if self.alert_clicks.1.try_iter().count() > 0 {
            show_window(ctx);
        }
        // eframe shows the window after the first frame, then applies this frame's commands.
        if std::mem::take(&mut self.start_in_tray) && self.has_tray() {
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        }
        // With a tray icon, closing the window only hides it; Quit is in the tray menu.
        if ctx.input(|i| i.viewport().close_requested()) && self.has_tray() && !self.quitting {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            egui::Popup::close_all(ctx);
        }
        if ctx
            .data_mut(|d| d.remove_temp::<bool>(ChartsWindow::request_id()))
            .is_some()
        {
            self.open_charts(ctx);
        }
        self.poll_fetch();
        self.poll_sky();
        self.poll_map();
        if let Some(charts) = &mut self.charts {
            charts.poll();
        }
        if self.last_fetch.elapsed() >= self.refresh_interval() {
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
                                dashboard(ui, data, &self.map, self.has_tray());
                            });
                    });
            });

        self.show_charts(ui.ctx());
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string(ALERTS_KEY, self.alerts.settings());
        if let Some(placement) = self.charts_placement {
            storage.set_string(CHARTS_PLACEMENT_KEY, placement.to_text());
        }
    }
}

fn show_window(ctx: &egui::Context) {
    ctx.send_viewport_cmd(ViewportCommand::Visible(true));
    ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd(ViewportCommand::Focus);
}

impl MeteoApp {
    fn open_charts(&mut self, ctx: &egui::Context) {
        match &mut self.charts {
            Some(_) => {
                ctx.send_viewport_cmd_to(ChartsWindow::viewport_id(), ViewportCommand::Focus)
            }
            None => {
                let mut charts = ChartsWindow::new(
                    self.charts_placement,
                    ctx.input(|i| i.viewport().monitor_size),
                    ctx.pixels_per_point(),
                );
                charts.load(ctx);
                self.charts = Some(charts);
            }
        }
    }

    fn show_charts(&mut self, ctx: &egui::Context) {
        let Some(charts) = &self.charts else { return };
        let mut builder = egui::ViewportBuilder::default()
            .with_title("MeteoCraft – Last 24 hours")
            .with_inner_size(charts.size)
            .with_min_inner_size([480.0, 400.0]);
        if let Some(position) = charts.position {
            builder = builder.with_position(position);
        }
        let (close, placement) =
            ctx.show_viewport_immediate(ChartsWindow::viewport_id(), builder, |ui, _| {
                egui::CentralPanel::default()
                    .frame(
                        egui::Frame::new()
                            .fill(theme::BG)
                            .inner_margin(Margin::symmetric(24, 18)),
                    )
                    .show(ui, |ui| charts.ui(ui));
                ui.input(|i| {
                    let viewport = i.viewport();
                    // A minimized or maximized window has no position worth reopening at.
                    let normal =
                        viewport.minimized != Some(true) && viewport.maximized != Some(true);
                    let placement = viewport
                        .outer_rect
                        .zip(viewport.inner_rect)
                        .filter(|_| normal)
                        .map(|(outer, inner)| Placement {
                            position: outer.min,
                            size: inner.size(),
                        });
                    (viewport.close_requested(), placement)
                })
            });
        if placement.is_some() {
            self.charts_placement = placement;
        }
        if close {
            self.charts = None;
        }
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

fn dashboard(ui: &mut egui::Ui, data: &StationData, map: &Map, has_tray: bool) {
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
        Box::new(move |ui| map_card(ui, map, has_tray)),
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
            ui.label(
                RichText::new(conditions(data))
                    .size(17.0)
                    .color(theme::TEXT),
            );
            ui.label(muted(format!(
                "Feels like {}",
                value_of(data.reading("občutek mraza"))
            )));
            ui.label(muted(format!(
                "Dew point {}",
                value_of(data.reading("rosišče"))
            )));
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
                    ui.label(
                        RichText::new(format!("{kmh:.1} km/h"))
                            .size(28.0)
                            .color(theme::TEXT),
                    );
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
        .and_then(|s| {
            Some(format!(
                "today {} – {}",
                s.min.as_deref()?,
                s.max.as_deref()?
            ))
        })
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
        .and_then(|s| {
            Some(format!(
                "today {} – {}",
                s.min.as_deref()?,
                s.max.as_deref()?
            ))
        })
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
        &format!(
            "last 24 h: {}",
            value_of(data.precipitation("padavine zadnjih 24h"))
        ),
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

/// Opens `url` in a window of its own and hides ours, as closing it would.
fn open_in_window(ctx: &egui::Context, url: &str, has_tray: bool) {
    if !app_window::open(url) {
        ctx.open_url(egui::OpenUrl::new_tab(url));
    }
    if has_tray {
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
    } else {
        ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
    }
}

fn map_card(ui: &mut egui::Ui, map: &Map, has_tray: bool) {
    card_title(ui, "🗺", "Koseze, Ljubljana");
    let width = ui.available_width();
    let (texture, attribution) = match map {
        Map::Ready(texture, attribution) => (texture, attribution),
        Map::Loading(_) => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(muted("Loading the map…"));
            });
            return;
        }
        Map::Failed(e) => {
            ui.label(muted(e.as_str()));
            return;
        }
    };
    let size = texture.size_vec2() * (width / texture.size_vec2().x);
    let image = egui::Image::new((texture.id(), size)).corner_radius(10);
    let response = ui.add(image.sense(Sense::click()));
    widgets::map_marker(ui.painter(), response.rect.center());
    if response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Open in Google Maps")
        .clicked()
    {
        open_in_window(ui.ctx(), &map::browser_url(), has_tray);
    }
    ui.label(muted(attribution.as_str()).size(11.0));
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
    if response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Open the last 24 hours as charts")
        .clicked()
    {
        ui.ctx()
            .data_mut(|d| d.insert_temp(ChartsWindow::request_id(), true));
        ui.ctx().request_repaint();
    }
}

/// Tray tooltip, e.g. "Ljubljana Koseze: 8.5 °C / Humidity 87% · wind 0.0 km/h WNW / …".
#[cfg(windows)]
fn tray_summary(data: &StationData, sky: Option<&Sky>) -> String {
    let wind = match data.wind.speed_kmh {
        Some(kmh) => format!(
            "{kmh:.1} km/h {}",
            data.wind
                .direction
                .split_whitespace()
                .next()
                .unwrap_or_default()
        ),
        None => data.wind.speed.clone(),
    };
    let sky = sky.map_or_else(String::new, |sky| format!(" · {}", sky.description));
    format!(
        "Ljubljana Koseze: {}{sky}\nHumidity {} · wind {wind}\nMeasured {}",
        value_of(data.reading("temperatura")),
        value_of(data.reading("vlažnost")),
        data.last_measurement.replace(" ob ", " at "),
    )
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
    let raining =
        number(value_of(data.precipitation("padavine v zadnji uri"))).is_some_and(|mm| mm > 0.0);
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
