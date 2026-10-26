//! A widget inside the taskbar, just left of the notification area, showing the
//! weather symbol beside the temperature at the taskbar's full height.
//!
//! The widget is a child window of Explorer's taskbar, which ties its input to
//! Explorer's, so it runs on a thread of its own that never does anything slow.

use std::cell::RefCell;
use std::ptr::{null, null_mut};
use std::sync::mpsc;
use std::thread::JoinHandle;

use ab_glyph::FontVec;
use eframe::egui::Color32;
use tray_icon::dpi::{PhysicalPosition, PhysicalSize};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE as Extent, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, ReleaseDC,
    ScreenToClient, SelectObject,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    DispatchMessageW, FindWindowExW, FindWindowW, GWL_STYLE, GetCursorPos, GetMessageW, GetParent,
    GetWindowRect, HWND_TOP, IDC_ARROW, IsWindow, LoadCursorW, MA_NOACTIVATE, MF_CHECKED,
    MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MSG, PostMessageW, PostQuitMessage, RegisterClassExW,
    RegisterWindowMessageW, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetForegroundWindow, SetParent,
    SetTimer, SetWindowLongPtrW, SetWindowPos, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx,
    TranslateMessage, ULW_ALPHA, UpdateLayeredWindow, WM_APP, WM_CLOSE, WM_DESTROY, WM_LBUTTONUP,
    WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_NULL, WM_RBUTTONUP, WM_TIMER, WNDCLASSEXW, WS_CHILD,
    WS_CLIPSIBLINGS, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows_sys::w;

use super::tooltip::Handle;
use super::{Command, Event, autostart, dark_mode};
use crate::sky::{self, Sky};

const WM_CONTENT: u32 = WM_APP + 1;
/// From `Win32_UI_Controls`, which is otherwise unused.
const WM_MOUSELEAVE: u32 = 0x02A3;
/// Explorer moves the notification area as icons come and go, and restarts with a
/// new taskbar, so the widget checks its place regularly.
const LAYOUT_TIMER: usize = 1;
const LAYOUT_INTERVAL_MS: u32 = 500;

const MENU_OPEN: usize = 1;
const MENU_REFRESH: usize = 2;
const MENU_AUTOSTART: usize = 3;
const MENU_QUIT: usize = 4;

pub struct Widget {
    controller: usize,
    thread: Option<JoinHandle<()>>,
}

impl Widget {
    pub fn spawn(notify: impl Fn(Event) + Send + 'static, tooltip: Option<Handle>) -> Option<Self> {
        let (ready_tx, ready) = mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("taskbar-widget".into())
            .spawn(move || run(Box::new(notify), tooltip, ready_tx))
            .ok()?;
        let controller = ready.recv().ok().flatten()?;
        Some(Self {
            controller,
            thread: Some(thread),
        })
    }

    pub fn update(&self, celsius: Option<f32>, sky: Option<&Sky>) {
        let content = Box::into_raw(Box::new(Content {
            celsius,
            sky: sky.cloned(),
        }));
        unsafe {
            if PostMessageW(self.controller as HWND, WM_CONTENT, 0, content as LPARAM) == 0 {
                drop(Box::from_raw(content));
            }
        }
    }
}

impl Drop for Widget {
    fn drop(&mut self) {
        unsafe { PostMessageW(self.controller as HWND, WM_CLOSE, 0, 0) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Default)]
struct Content {
    celsius: Option<f32>,
    sky: Option<Sky>,
}

struct State {
    /// Hidden top-level window: receives updates and Explorer's broadcasts, and owns
    /// the menu.
    controller: HWND,
    /// The child window in the taskbar, or null while it isn't there.
    widget: HWND,
    taskbar_created: u32,
    content: Content,
    /// Bumped with every update, so the widget knows to redraw.
    version: u64,
    font: Option<FontVec>,
    hovered: bool,
    /// Height, hover state and content version of the last drawing, and its width.
    drawn: Option<(usize, bool, u64)>,
    width: usize,
    /// Position and size in taskbar coordinates.
    placed: Option<(i32, i32, i32, i32)>,
    embedded: bool,
    /// Explorer refused a layered child window; the tray icon stays instead.
    unsupported: bool,
    notify: Box<dyn Fn(Event) + Send>,
    tooltip: Option<Handle>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Window procedures re-enter while Win32 calls dispatch messages, so the state is
/// only borrowed briefly and never across such calls.
fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|state| state.try_borrow_mut().ok()?.as_mut().map(f))
}

fn run(
    notify: Box<dyn Fn(Event) + Send>,
    tooltip: Option<Handle>,
    ready: mpsc::SyncSender<Option<usize>>,
) {
    unsafe {
        let instance = GetModuleHandleW(null());
        let controller_class = w!("MeteoCraftWidgetController");
        RegisterClassExW(&WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(controller_proc),
            hInstance: instance,
            lpszClassName: controller_class,
            ..std::mem::zeroed()
        });
        RegisterClassExW(&WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(widget_proc),
            hInstance: instance,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: WIDGET_CLASS,
            ..std::mem::zeroed()
        });
        let controller = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            controller_class,
            null(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if controller.is_null() {
            let _ = ready.send(None);
            return;
        }
        STATE.with(|state| {
            *state.borrow_mut() = Some(State {
                controller,
                widget: null_mut(),
                taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
                content: Content::default(),
                version: 0,
                font: crate::theme::bold_font().and_then(|bytes| FontVec::try_from_vec(bytes).ok()),
                hovered: false,
                drawn: None,
                width: 0,
                placed: None,
                embedded: false,
                unsupported: false,
                notify,
                tooltip,
            });
        });
        SetTimer(controller, LAYOUT_TIMER, LAYOUT_INTERVAL_MS, None);
        let _ = ready.send(Some(controller as usize));
        layout();

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        STATE.with(|state| state.borrow_mut().take());
    }
}

const WIDGET_CLASS: *const u16 = w!("MeteoCraftTaskbarWidget");

unsafe extern "system" fn controller_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match msg {
            WM_CONTENT => {
                let content = *Box::from_raw(lparam as *mut Content);
                with_state(|s| {
                    s.content = content;
                    s.version += 1;
                });
                layout();
                0
            }
            WM_TIMER if wparam == LAYOUT_TIMER => {
                layout();
                0
            }
            WM_CLOSE => {
                if let Some(widget) = with_state(|s| std::mem::replace(&mut s.widget, null_mut()))
                    && !widget.is_null()
                {
                    DestroyWindow(widget);
                }
                DestroyWindow(hwnd);
                0
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            _ if with_state(|s| s.taskbar_created) == Some(msg) => {
                layout();
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

unsafe extern "system" fn widget_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match msg {
            WM_MOUSEMOVE => {
                if with_state(|s| std::mem::replace(&mut s.hovered, true)) == Some(false) {
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    TrackMouseEvent(&mut track);
                    layout();
                }
                let mut rect: RECT = std::mem::zeroed();
                if let Some(Some(tip)) = with_state(|s| s.tooltip)
                    && GetWindowRect(hwnd, &mut rect) != 0
                {
                    tip.hover(&tray_icon::Rect {
                        position: PhysicalPosition::new(f64::from(rect.left), f64::from(rect.top)),
                        size: PhysicalSize::new(
                            (rect.right - rect.left) as u32,
                            (rect.bottom - rect.top) as u32,
                        ),
                    });
                }
                0
            }
            WM_MOUSELEAVE => {
                with_state(|s| s.hovered = false);
                layout();
                0
            }
            WM_LBUTTONUP => {
                hide_tooltip();
                send(Event::Command(Command::Show));
                0
            }
            WM_RBUTTONUP => {
                hide_tooltip();
                show_menu();
                0
            }
            WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
            WM_DESTROY => {
                with_state(|s| {
                    if s.widget == hwnd {
                        s.widget = null_mut();
                    }
                });
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

fn send(event: Event) {
    with_state(|s| (s.notify)(event));
}

fn hide_tooltip() {
    if let Some(Some(tip)) = with_state(|s| s.tooltip) {
        tip.hide();
    }
}

/// Puts the widget into the taskbar if it isn't there, redraws it if anything
/// changed and moves it next to the notification area.
fn layout() {
    let Some((widget, unsupported)) = with_state(|s| (s.widget, s.unsupported)) else {
        return;
    };
    if unsupported {
        return;
    }
    unsafe {
        let taskbar = FindWindowW(w!("Shell_TrayWnd"), null());
        let notify_area = if taskbar.is_null() {
            null_mut()
        } else {
            FindWindowExW(taskbar, null_mut(), w!("TrayNotifyWnd"), null())
        };
        let mut bar: RECT = std::mem::zeroed();
        let mut area: RECT = std::mem::zeroed();
        let horizontal = !notify_area.is_null()
            && GetWindowRect(taskbar, &mut bar) != 0
            && GetWindowRect(notify_area, &mut area) != 0
            && bar.right - bar.left > bar.bottom - bar.top;
        if !horizontal {
            remove(widget);
            set_embedded(false);
            return;
        }

        let mut widget = widget;
        if widget.is_null() || IsWindow(widget) == 0 || GetParent(widget) != taskbar {
            remove(widget);
            widget = create(taskbar);
            with_state(|s| {
                s.widget = widget;
                s.drawn = None;
                s.placed = None;
            });
            if widget.is_null() {
                set_embedded(false);
                return;
            }
        }

        let height = (bar.bottom - bar.top) as usize;
        let drawing = with_state(|s| {
            let key = (height, s.hovered, s.version);
            (s.drawn != Some(key)).then(|| {
                s.drawn = Some(key);
                draw(&s.content, height, s.hovered, s.font.as_ref())
            })
        })
        .flatten();
        if let Some((width, pixels)) = drawing {
            if !present(widget, width, height, &pixels) {
                remove(widget);
                with_state(|s| s.unsupported = true);
                set_embedded(false);
                return;
            }
            with_state(|s| s.width = width);
        }

        let width = with_state(|s| s.width).unwrap_or_default() as i32;
        let gap = (height / 12) as i32;
        let mut at = POINT {
            x: area.left - gap - width,
            y: bar.top,
        };
        ScreenToClient(taskbar, &mut at);
        let place = (at.x, at.y, width, height as i32);
        if with_state(|s| s.placed.replace(place) != Some(place)) == Some(true) {
            SetWindowPos(
                widget,
                HWND_TOP,
                place.0,
                place.1,
                place.2,
                place.3,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
        set_embedded(true);
    }
}

unsafe fn create(taskbar: HWND) -> HWND {
    unsafe {
        let widget = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            WIDGET_CLASS,
            w!("MeteoCraft"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            GetModuleHandleW(null()),
            null(),
        );
        if widget.is_null() {
            return widget;
        }
        SetWindowLongPtrW(widget, GWL_STYLE, (WS_CHILD | WS_CLIPSIBLINGS) as isize);
        SetParent(widget, taskbar);
        if GetParent(widget) != taskbar {
            DestroyWindow(widget);
            return null_mut();
        }
        widget
    }
}

unsafe fn remove(widget: HWND) {
    unsafe {
        if !widget.is_null() && IsWindow(widget) != 0 {
            DestroyWindow(widget);
        }
    }
}

fn set_embedded(embedded: bool) {
    with_state(|s| {
        if s.embedded != embedded {
            s.embedded = embedded;
            (s.notify)(Event::Embedded(embedded));
        }
    });
}

/// Hands the premultiplied BGRA pixels to the layered window.
unsafe fn present(widget: HWND, width: usize, height: usize, pixels: &[u8]) -> bool {
    unsafe {
        let screen = GetDC(null_mut());
        let memory = CreateCompatibleDC(screen);
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            },
            ..std::mem::zeroed()
        };
        let mut bits = null_mut();
        let bitmap = CreateDIBSection(memory, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        let mut presented = false;
        if !bitmap.is_null() {
            std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast(), pixels.len());
            let previous = SelectObject(memory, bitmap);
            let size = Extent {
                cx: width as i32,
                cy: height as i32,
            };
            let origin = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            presented = UpdateLayeredWindow(
                widget,
                screen,
                null(),
                &size,
                memory,
                &origin,
                0,
                &blend,
                ULW_ALPHA,
            ) != 0;
            SelectObject(memory, previous);
            DeleteObject(bitmap);
        }
        DeleteDC(memory);
        ReleaseDC(null_mut(), screen);
        presented
    }
}

fn show_menu() {
    let Some(controller) = with_state(|s| s.controller) else {
        return;
    };
    let chosen = unsafe {
        let menu = CreatePopupMenu();
        let checked = if autostart::is_enabled() {
            MF_CHECKED
        } else {
            MF_UNCHECKED
        };
        AppendMenuW(menu, MF_STRING, MENU_OPEN, w!("Open MeteoCraft"));
        AppendMenuW(menu, MF_STRING, MENU_REFRESH, w!("Refresh now"));
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        AppendMenuW(
            menu,
            MF_STRING | checked,
            MENU_AUTOSTART,
            w!("Start with Windows"),
        );
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        AppendMenuW(menu, MF_STRING, MENU_QUIT, w!("Quit"));
        dark_mode::theme_menus();

        let mut at = POINT { x: 0, y: 0 };
        GetCursorPos(&mut at);
        // Without this the menu doesn't close when the user clicks elsewhere.
        SetForegroundWindow(controller);
        let chosen = TrackPopupMenuEx(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            at.x,
            at.y,
            controller,
            null(),
        ) as usize;
        PostMessageW(controller, WM_NULL, 0, 0);
        DestroyMenu(menu);
        chosen
    };
    match chosen {
        MENU_OPEN => send(Event::Command(Command::Show)),
        MENU_REFRESH => send(Event::Command(Command::Refresh)),
        MENU_AUTOSTART => {
            if let Err(e) = autostart::set(!autostart::is_enabled()) {
                eprintln!("could not change the sign-in setting: {e}");
            }
            send(Event::AutostartChanged);
        }
        MENU_QUIT => send(Event::Command(Command::Quit)),
        _ => {}
    }
}

/// The widget's pixels as premultiplied BGRA, and its width: the symbol, then the
/// temperature, sized to the taskbar's `height`.
fn draw(
    content: &Content,
    height: usize,
    hovered: bool,
    font: Option<&FontVec>,
) -> (usize, Vec<u8>) {
    let h = height as f32;
    let pad = (h * 0.14).round() as usize;
    let gap = (h * 0.08).round() as usize;
    let symbol_size = (h * 0.62).round() as usize;

    let (text, color) = super::temperature_label(content.celsius);
    let glyphs = font
        .map(|font| {
            const PROBE: f32 = 100.0;
            let (_, probe_height) = super::extent(&super::layout(font, &text, PROBE));
            super::layout(font, &text, PROBE * h * 0.42 / probe_height)
        })
        .unwrap_or_default();
    let bounds = super::union_bounds(&glyphs);
    let text_width = bounds.map_or(0, |b| b.width().ceil() as usize);
    let symbol_width = if content.sky.is_some() {
        symbol_size + gap
    } else {
        0
    };
    let width = pad + symbol_width + text_width + pad;

    // Nearly transparent everywhere, so the whole widget takes clicks.
    let mut rgba = [0_u8, 0, 0, 1].repeat(width * height);
    if hovered {
        let (tint, alpha) = if dark_mode::system_is_dark() {
            (Color32::WHITE, 0.1)
        } else {
            (Color32::BLACK, 0.07)
        };
        let inset = h * 0.1;
        let radius = h * 0.12;
        for y in 0..height {
            for x in 0..width {
                let p = (x as f32 + 0.5, y as f32 + 0.5);
                let d =
                    rounded_rect_distance(p, (1.0, inset), (width as f32 - 1.0, h - inset), radius);
                let pixel = &mut rgba[(y * width + x) * 4..][..4];
                super::over(pixel, tint, (0.5 - d).clamp(0.0, 1.0) * alpha);
            }
        }
    }

    if let Some(sky) = &content.sky {
        let symbol = sky::icon::render(sky, symbol_size, h / 40.0);
        let top = (height - symbol_size) / 2;
        for (i, source) in symbol.chunks(4).enumerate() {
            let (x, y) = (pad + i % symbol_size, top + i / symbol_size);
            let pixel = &mut rgba[(y * width + x) * 4..][..4];
            let color = Color32::from_rgb(source[0], source[1], source[2]);
            super::over(pixel, color, f32::from(source[3]) / 255.0);
        }
    }

    if let Some(bounds) = bounds {
        let mut coverage = vec![0.0; width * height];
        let at = (
            (pad + symbol_width) as f32 - bounds.min.x,
            ((h - bounds.height()) / 2.0 - bounds.min.y).round(),
        );
        super::stamp(&glyphs, at, &mut coverage, width);
        super::paint_text(&mut rgba, &coverage, width, color);
    }

    for pixel in rgba.chunks_mut(4) {
        let alpha = u16::from(pixel[3]);
        let premultiply = |c: u8| ((u16::from(c) * alpha + 127) / 255) as u8;
        let (r, g, b) = (pixel[0], pixel[1], pixel[2]);
        pixel[0] = premultiply(b);
        pixel[1] = premultiply(g);
        pixel[2] = premultiply(r);
    }
    (width, rgba)
}

fn rounded_rect_distance(p: (f32, f32), min: (f32, f32), max: (f32, f32), radius: f32) -> f32 {
    let center = ((min.0 + max.0) / 2.0, (min.1 + max.1) / 2.0);
    let half = (
        (max.0 - min.0) / 2.0 - radius,
        (max.1 - min.1) / 2.0 - radius,
    );
    let q = (
        ((p.0 - center.0).abs() - half.0).max(0.0),
        ((p.1 - center.1).abs() - half.1).max(0.0),
    );
    (q.0 * q.0 + q.1 * q.1).sqrt() - radius
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sky::Cover;

    #[test]
    fn draws_symbol_and_temperature_side_by_side() {
        let Some(font) = crate::theme::bold_font().and_then(|b| FontVec::try_from_vec(b).ok())
        else {
            return;
        };
        let content = Content {
            celsius: Some(-12.0),
            sky: Some(Sky {
                cover: Cover::Clear,
                precipitation: None,
                thunder: false,
                showers: false,
                night: false,
                description: String::new(),
                observed: String::new(),
            }),
        };
        let height = 80;
        let (width, pixels) = draw(&content, height, false, Some(&font));
        assert_eq!(pixels.len(), width * height * 4);
        assert!(width > height * 3 / 2, "only {width} px wide");
        // Every pixel takes clicks.
        assert!(pixels.chunks(4).all(|p| p[3] >= 1));
        let without_sky = draw(
            &Content {
                sky: None,
                ..content
            },
            height,
            false,
            Some(&font),
        )
        .0;
        assert!(without_sky < width);
    }
}
