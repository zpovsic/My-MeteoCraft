//! The taskbar widget showing the current weather and temperature, with a
//! notification-area ("system tray") icon of the temperature as its fallback.

use std::error::Error;
use std::sync::mpsc::{self, Receiver};

use ab_glyph::{Font as _, FontVec, OutlinedGlyph, PxScale, ScaleFont as _, point};
use eframe::egui::{self, Color32};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{
    BadIcon, Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};

use crate::sky::Sky;
use crate::theme;

mod autostart;
mod dark_mode;
mod tooltip;
mod widget;

pub use autostart::TRAY_ARG;
use tooltip::Tooltip;
use widget::Widget;

/// Icon edge in pixels; Windows scales it to the taskbar's DPI.
const SIZE: usize = 32;
const HALO: Color32 = Color32::from_rgb(10, 14, 22);

pub enum Command {
    Show,
    Refresh,
    Quit,
}

/// What the tray's event handlers and the widget report; all but `Command` are
/// handled by the tray itself.
enum Event {
    Command(Command),
    ToggleAutostart,
    /// The widget changed the sign-in setting from its own menu.
    AutostartChanged,
    /// Whether the widget is in the taskbar; the tray icon stands in when it isn't.
    Embedded(bool),
}

pub struct Tray {
    icon: TrayIcon,
    widget: Option<Widget>,
    tooltip: Option<Tooltip>,
    font: Option<FontVec>,
    autostart: CheckMenuItem,
    events: Receiver<Event>,
}

impl Tray {
    /// Must be called on the thread that runs the window's event loop.
    pub fn new(ctx: &egui::Context) -> Result<Self, Box<dyn Error>> {
        let (tx, events) = mpsc::channel();

        let open = MenuItem::new("Open MeteoCraft", true, None);
        let refresh = MenuItem::new("Refresh now", true, None);
        let autostart =
            CheckMenuItem::new("Start with Windows", true, autostart::is_enabled(), None);
        let quit = MenuItem::new("Quit", true, None);
        let menu = Menu::with_items(&[
            &open,
            &refresh,
            &PredefinedMenuItem::separator(),
            &autostart,
            &PredefinedMenuItem::separator(),
            &quit,
        ])?;

        // Both handlers run on the event-loop thread while the window may be hidden,
        // so they wake egui up to process the event.
        let (open_id, refresh_id, autostart_id, quit_id) = (
            open.id().clone(),
            refresh.id().clone(),
            autostart.id().clone(),
            quit.id().clone(),
        );
        let (menu_tx, menu_ctx) = (tx.clone(), ctx.clone());
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let event = if event.id == open_id {
                Event::Command(Command::Show)
            } else if event.id == refresh_id {
                Event::Command(Command::Refresh)
            } else if event.id == autostart_id {
                Event::ToggleAutostart
            } else if event.id == quit_id {
                Event::Command(Command::Quit)
            } else {
                return;
            };
            let _ = menu_tx.send(event);
            menu_ctx.request_repaint();
        }));

        // The icon gets no shell tooltip: the app's own `Tooltip` replaces it.
        let tooltip = Tooltip::new();
        if let Some(tooltip) = &tooltip {
            tooltip.set_text("MeteoCraft – loading…");
        }
        let tip = tooltip.as_ref().map(Tooltip::handle);
        let (widget_tx, widget_ctx) = (tx.clone(), ctx.clone());
        let widget = Widget::spawn(
            move |event| {
                let _ = widget_tx.send(event);
                widget_ctx.request_repaint();
            },
            tip,
        );
        let click_ctx = ctx.clone();
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| match event {
            TrayIconEvent::Enter { rect, .. } | TrayIconEvent::Move { rect, .. } => {
                if let Some(tip) = tip {
                    tip.hover(&rect);
                }
            }
            TrayIconEvent::Leave { .. } => {
                if let Some(tip) = tip {
                    tip.hide();
                }
            }
            TrayIconEvent::Click {
                button,
                button_state,
                ..
            } => {
                if let Some(tip) = tip {
                    tip.hide();
                }
                match (button, button_state) {
                    // The menu opens on release; the theme may have changed since the last time.
                    (MouseButton::Right, MouseButtonState::Down) => dark_mode::theme_menus(),
                    (MouseButton::Left, MouseButtonState::Up) => {
                        let _ = tx.send(Event::Command(Command::Show));
                        click_ctx.request_repaint();
                    }
                    _ => {}
                }
            }
            _ => {}
        }));

        let font = theme::bold_font().and_then(|bytes| FontVec::try_from_vec(bytes).ok());
        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_icon(render(font.as_ref(), None)?)
            .build()?;

        Ok(Self {
            icon,
            widget,
            tooltip,
            font,
            autostart,
            events,
        })
    }

    pub fn update(&self, celsius: Option<f32>, sky: Option<&Sky>, tooltip: &str) {
        if let Ok(icon) = render(self.font.as_ref(), celsius) {
            let _ = self.icon.set_icon(Some(icon));
        }
        if let Some(widget) = &self.widget {
            widget.update(celsius, sky);
        }
        if let Some(tip) = &self.tooltip {
            tip.set_text(tooltip);
        }
    }

    pub fn poll(&self) -> Option<Command> {
        loop {
            match self.events.try_recv().ok()? {
                Event::Command(command) => return Some(command),
                Event::ToggleAutostart => {
                    // The menu has already flipped the check mark; make it match the registry.
                    if let Err(e) = autostart::set(self.autostart.is_checked()) {
                        eprintln!("could not change the sign-in setting: {e}");
                    }
                    self.autostart.set_checked(autostart::is_enabled());
                }
                Event::AutostartChanged => self.autostart.set_checked(autostart::is_enabled()),
                Event::Embedded(embedded) => {
                    let _ = self.icon.set_visible(!embedded);
                }
            }
        }
    }
}

/// The temperature as shown on the taskbar, e.g. "8°", in its color.
fn temperature_label(celsius: Option<f32>) -> (String, Color32) {
    match celsius {
        Some(t) => (
            format!("{}°", t.round() as i32),
            theme::temperature_color(t),
        ),
        None => ("--".to_owned(), theme::MUTED),
    }
}

fn render(font: Option<&FontVec>, celsius: Option<f32>) -> Result<Icon, BadIcon> {
    let (text, color) = temperature_label(celsius);
    let mut coverage = vec![0.0_f32; SIZE * SIZE];
    match font {
        Some(font) => draw_text(font, &text, &mut coverage),
        None => draw_disk(&mut coverage),
    }
    let mut rgba = vec![0_u8; SIZE * SIZE * 4];
    paint_text(&mut rgba, &coverage, SIZE, color);
    Icon::from_rgba(rgba, SIZE as u32, SIZE as u32)
}

/// Draws `text` as large as fits, centered. Drops the degree sign if keeping it
/// would make the digits too small to read (e.g. "-12°").
fn draw_text(font: &FontVec, text: &str, coverage: &mut [f32]) {
    let usable = (SIZE - 2) as f32;
    let fit = |text: &str| {
        const PROBE: f32 = 40.0;
        let (w, h) = extent(&layout(font, text, PROBE));
        PROBE * (usable / w).min(usable / h)
    };
    let mut text = text;
    let mut scale = fit(text);
    let digits_only = text.trim_end_matches('°');
    if scale < usable && digits_only != text {
        text = digits_only;
        scale = fit(text);
    }

    let glyphs = layout(font, text, scale);
    let Some(bounds) = union_bounds(&glyphs) else {
        return;
    };
    let dx = ((SIZE as f32 - bounds.width()) / 2.0 - bounds.min.x).round();
    let dy = ((SIZE as f32 - bounds.height()) / 2.0 - bounds.min.y).round();
    stamp(&glyphs, (dx, dy), coverage, SIZE);
}

/// Adds the glyphs, offset by `at`, to a coverage map `width` pixels wide.
fn stamp(glyphs: &[OutlinedGlyph], at: (f32, f32), coverage: &mut [f32], width: usize) {
    let height = coverage.len() / width;
    for glyph in glyphs {
        let b = glyph.px_bounds();
        glyph.draw(|x, y, c| {
            let px = (b.min.x + at.0) as i32 + x as i32;
            let py = (b.min.y + at.1) as i32 + y as i32;
            if (0..width as i32).contains(&px) && (0..height as i32).contains(&py) {
                let cell = &mut coverage[py as usize * width + px as usize];
                *cell = cell.max(c);
            }
        });
    }
}

fn layout(font: &FontVec, text: &str, scale: f32) -> Vec<OutlinedGlyph> {
    let scaled = font.as_scaled(PxScale::from(scale));
    let mut x = 0.0;
    let mut previous = None;
    let mut glyphs = Vec::new();
    for c in text.chars() {
        let id = scaled.glyph_id(c);
        if let Some(prev) = previous {
            x += scaled.kern(prev, id);
        }
        let glyph = id.with_scale_and_position(scale, point(x, scaled.ascent()));
        x += scaled.h_advance(id);
        previous = Some(id);
        glyphs.extend(font.outline_glyph(glyph));
    }
    glyphs
}

fn union_bounds(glyphs: &[OutlinedGlyph]) -> Option<ab_glyph::Rect> {
    glyphs
        .iter()
        .map(OutlinedGlyph::px_bounds)
        .reduce(|a, b| ab_glyph::Rect {
            min: point(a.min.x.min(b.min.x), a.min.y.min(b.min.y)),
            max: point(a.max.x.max(b.max.x), a.max.y.max(b.max.y)),
        })
}

fn extent(glyphs: &[OutlinedGlyph]) -> (f32, f32) {
    union_bounds(glyphs).map_or((1.0, 1.0), |b| (b.width().max(1.0), b.height().max(1.0)))
}

/// Fallback when no system font is available: a plain colored dot.
fn draw_disk(coverage: &mut [f32]) {
    let c = SIZE as f32 / 2.0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let d = ((x as f32 + 0.5 - c).powi(2) + (y as f32 + 0.5 - c).powi(2)).sqrt();
            coverage[y * SIZE + x] = (11.0 - d).clamp(0.0, 1.0);
        }
    }
}

/// Paints text in `color` over a 1 px dark outline onto `rgba` (unpremultiplied,
/// `width` pixels wide), so it reads on light and dark taskbars.
fn paint_text(rgba: &mut [u8], coverage: &[f32], width: usize, color: Color32) {
    let (w, h) = (width as i32, (coverage.len() / width) as i32);
    let at = |x: i32, y: i32| {
        if (0..w).contains(&x) && (0..h).contains(&y) {
            coverage[(y * w + x) as usize]
        } else {
            0.0
        }
    };
    for y in 0..h {
        for x in 0..w {
            let text = at(x, y);
            let halo = (-1..=1)
                .flat_map(|dy| (-1..=1).map(move |dx| (dx, dy)))
                .map(|(dx, dy)| at(x + dx, y + dy))
                .fold(0.0, f32::max)
                * 0.9;
            let pixel = &mut rgba[((y * w + x) * 4) as usize..][..4];
            over(pixel, HALO, halo * (1.0 - text));
            over(pixel, color, text);
        }
    }
}

fn over(pixel: &mut [u8], color: Color32, alpha: f32) {
    if alpha <= 0.0 {
        return;
    }
    let below = f32::from(pixel[3]) / 255.0 * (1.0 - alpha);
    let total = alpha + below;
    for (channel, value) in pixel.iter_mut().zip([color.r(), color.g(), color.b()]) {
        *channel = ((f32::from(value) * alpha + f32::from(*channel) * below) / total).round() as u8;
    }
    pixel[3] = (total * 255.0).round() as u8;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn system_font() -> Option<FontVec> {
        theme::bold_font().and_then(|b| FontVec::try_from_vec(b).ok())
    }

    fn inked_width(coverage: &[f32]) -> usize {
        let cols: Vec<usize> = (0..SIZE)
            .filter(|&x| (0..SIZE).any(|y| coverage[y * SIZE + x] > 0.5))
            .collect();
        cols.last().map_or(0, |last| last - cols[0] + 1)
    }

    #[test]
    fn text_fits_inside_the_icon() {
        let Some(font) = system_font() else { return };
        for text in ["8°", "23°", "-12°", "--"] {
            let mut coverage = vec![0.0; SIZE * SIZE];
            draw_text(&font, text, &mut coverage);
            let width = inked_width(&coverage);
            assert!(
                width > 10 && width <= SIZE - 2,
                "{text:?} is {width} px wide"
            );
        }
    }
}
