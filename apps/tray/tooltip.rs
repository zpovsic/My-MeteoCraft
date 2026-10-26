//! Hover tooltip for the tray icon. The shell draws tray tooltips in the light
//! style regardless of the theme, so the app draws its own to match the taskbar.

use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, TRUE, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{
    DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUNDSMALL, DwmSetWindowAttribute,
};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontIndirectW, CreateSolidBrush, DT_CALCRECT, DT_NOPREFIX, DeleteObject,
    DrawTextW, EndPaint, FillRect, FrameRect, GetDC, GetMonitorInfoW, HFONT, InvalidateRect,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, PAINTSTRUCT, ReleaseDC, SelectObject,
    SetBkMode, SetTextColor, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{
    GetDpiForMonitor, MDT_EFFECTIVE_DPI, SystemParametersInfoForDpi,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CS_DROPSHADOW, CreateWindowExW, DefWindowProcW, DestroyWindow,
    GUI_POPUPMENUMODE, GUITHREADINFO, GWLP_USERDATA, GetClientRect, GetCursorPos, GetGUIThreadInfo,
    GetWindowLongPtrW, GetWindowThreadProcessId, HTTRANSPARENT, HWND_TOPMOST, KillTimer,
    NONCLIENTMETRICSW, PostMessageW, RegisterClassExW, SPI_GETNONCLIENTMETRICS, SW_HIDE,
    SWP_NOACTIVATE, SWP_SHOWWINDOW, SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow, WM_APP,
    WM_NCCREATE, WM_NCDESTROY, WM_NCHITTEST, WM_PAINT, WM_TIMER, WNDCLASSEXW, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows_sys::w;

use super::dark_mode;

const WM_HOVER: u32 = WM_APP + 1;
const WM_LEAVE: u32 = WM_APP + 2;
const SHOW_TIMER: usize = 1;
const SHOW_DELAY_MS: u32 = 400;
/// The tray doesn't report every exit from the icon, so the cursor is also polled.
const WATCH_TIMER: usize = 2;
const WATCH_INTERVAL_MS: u32 = 100;
/// Text padding and distance from the icon, in pixels at 100% scaling.
const PADDING: (i32, i32) = (9, 6);
const GAP: i32 = 8;

struct Palette {
    background: COLORREF,
    border: COLORREF,
    text: COLORREF,
}

const DARK: Palette = Palette {
    background: rgb(43, 43, 43),
    border: rgb(72, 72, 72),
    text: rgb(255, 255, 255),
};
const LIGHT: Palette = Palette {
    background: rgb(249, 249, 249),
    border: rgb(214, 214, 214),
    text: rgb(26, 26, 26),
};

const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

struct State {
    text: Vec<u16>,
    /// Screen rectangle of the tray icon.
    anchor: RECT,
    font: HFONT,
    padding: (i32, i32),
    dark: bool,
    /// DWM draws the rounded corners and border (Windows 11).
    dwm_frame: bool,
    shown: bool,
}

/// A popup window owned by the thread that runs the tray icon.
pub struct Tooltip {
    hwnd: HWND,
}

/// Lets the tray event handler, which must be `Send + Sync`, reach the tooltip.
/// It only posts messages, so the tooltip is still handled on its own thread.
#[derive(Clone, Copy)]
pub struct Handle(usize);

impl Handle {
    /// Shows the tooltip after a short delay, like the system's tooltips, and keeps
    /// it up while the cursor stays on `icon`.
    pub fn hover(self, icon: &tray_icon::Rect) {
        let (left, top) = (
            icon.position.x.round() as i32,
            icon.position.y.round() as i32,
        );
        let anchor = Box::into_raw(Box::new(RECT {
            left,
            top,
            right: left + icon.size.width as i32,
            bottom: top + icon.size.height as i32,
        }));
        unsafe {
            if PostMessageW(self.0 as HWND, WM_HOVER, 0, anchor as LPARAM) == 0 {
                drop(Box::from_raw(anchor));
            }
        }
    }

    pub fn hide(self) {
        unsafe { PostMessageW(self.0 as HWND, WM_LEAVE, 0, 0) };
    }
}

impl Tooltip {
    pub fn new() -> Option<Self> {
        unsafe {
            let instance = GetModuleHandleW(null());
            let class_name = w!("MeteoCraftTrayTooltip");
            let class = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                style: CS_DROPSHADOW,
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
                lpszClassName: class_name,
                ..std::mem::zeroed()
            };
            RegisterClassExW(&class);

            let state = Box::into_raw(Box::new(State {
                text: Vec::new(),
                anchor: RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                },
                font: null_mut(),
                padding: (0, 0),
                dark: false,
                dwm_frame: false,
                shown: false,
            }));
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
                class_name,
                null(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                instance,
                state.cast(),
            );
            if hwnd.is_null() {
                drop(Box::from_raw(state));
                return None;
            }
            Some(Self { hwnd })
        }
    }

    pub fn handle(&self) -> Handle {
        Handle(self.hwnd as usize)
    }

    pub fn set_text(&self, text: &str) {
        unsafe {
            let Some(state) = state(self.hwnd) else {
                return;
            };
            state.text = text.encode_utf16().collect();
            if state.shown {
                show(self.hwnd, state);
            }
        }
    }
}

impl Drop for Tooltip {
    fn drop(&mut self) {
        unsafe { DestroyWindow(self.hwnd) };
    }
}

/// # Safety
/// Only call on the window's thread, and don't keep the reference across calls
/// that dispatch messages to the window.
unsafe fn state<'a>(hwnd: HWND) -> Option<&'a mut State> {
    unsafe { (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State).as_mut() }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let create = &*(lparam as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        match msg {
            WM_HOVER => {
                let anchor = *Box::from_raw(lparam as *mut RECT);
                if let Some(state) = state(hwnd) {
                    state.anchor = anchor;
                    if !state.shown {
                        // Restarting the timer keeps it from firing while the cursor moves.
                        SetTimer(hwnd, SHOW_TIMER, SHOW_DELAY_MS, None);
                    }
                    SetTimer(hwnd, WATCH_TIMER, WATCH_INTERVAL_MS, None);
                }
                0
            }
            WM_TIMER if wparam == SHOW_TIMER || wparam == WATCH_TIMER => {
                if let Some(state) = state(hwnd) {
                    if !cursor_within(&state.anchor) || menu_open(hwnd) {
                        hide(hwnd, state);
                    } else if wparam == SHOW_TIMER {
                        KillTimer(hwnd, SHOW_TIMER);
                        show(hwnd, state);
                    }
                }
                0
            }
            WM_LEAVE => {
                if let Some(state) = state(hwnd) {
                    hide(hwnd, state);
                }
                0
            }
            WM_PAINT => {
                if let Some(state) = state(hwnd) {
                    paint(hwnd, state);
                }
                0
            }
            WM_NCHITTEST => HTTRANSPARENT as LRESULT,
            WM_NCDESTROY => {
                let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                if !state.is_null() {
                    let state = Box::from_raw(state);
                    if !state.font.is_null() {
                        DeleteObject(state.font);
                    }
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// The tray menu runs on this window's thread and must not end up under the tooltip.
fn menu_open(hwnd: HWND) -> bool {
    let mut info = GUITHREADINFO {
        cbSize: size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    let found = unsafe {
        let thread = GetWindowThreadProcessId(hwnd, null_mut());
        GetGUIThreadInfo(thread, &mut info)
    };
    found != 0 && info.flags & GUI_POPUPMENUMODE != 0
}

/// Sizes the tooltip to its text and places it next to the taskbar under the cursor.
unsafe fn show(hwnd: HWND, state: &mut State) {
    if state.text.is_empty() {
        return;
    }
    unsafe {
        let anchor = state.anchor;
        let center = POINT {
            x: (anchor.left + anchor.right) / 2,
            y: (anchor.top + anchor.bottom) / 2,
        };
        let monitor = MonitorFromPoint(center, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..std::mem::zeroed()
        };
        GetMonitorInfoW(monitor, &mut info);
        let (mut dpi, mut dpi_y) = (96, 96);
        GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut dpi_y);
        let px = |v: i32| v * dpi as i32 / 96;

        let mut metrics = NONCLIENTMETRICSW {
            cbSize: size_of::<NONCLIENTMETRICSW>() as u32,
            ..std::mem::zeroed()
        };
        SystemParametersInfoForDpi(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            (&raw mut metrics).cast(),
            0,
            dpi,
        );
        if !state.font.is_null() {
            DeleteObject(state.font);
        }
        state.font = CreateFontIndirectW(&metrics.lfStatusFont);
        state.padding = (px(PADDING.0), px(PADDING.1));
        state.dark = dark_mode::system_is_dark();

        let mut text = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let dc = GetDC(hwnd);
        let previous = SelectObject(dc, state.font);
        DrawTextW(
            dc,
            state.text.as_ptr(),
            state.text.len() as i32,
            &mut text,
            DT_CALCRECT | DT_NOPREFIX,
        );
        SelectObject(dc, previous);
        ReleaseDC(hwnd, dc);
        let width = text.right + 2 * state.padding.0;
        let height = text.bottom + 2 * state.padding.1;

        // Below icons in the top half of the screen, above the rest. The work area
        // keeps the tooltip clear of a taskbar that doesn't auto-hide.
        let (screen, work, gap) = (info.rcMonitor, info.rcWork, px(GAP));
        let y = if center.y < (screen.top + screen.bottom) / 2 {
            anchor.bottom.max(work.top) + gap
        } else {
            anchor.top.min(work.bottom) - height - gap
        };
        let x = (center.x - width / 2).clamp(screen.left, (screen.right - width).max(screen.left));
        let y = y.clamp(screen.top, (screen.bottom - height).max(screen.top));

        let palette = if state.dark { &DARK } else { &LIGHT };
        let corners = DWMWCP_ROUNDSMALL;
        state.dwm_frame = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            (&raw const corners).cast(),
            size_of_val(&corners) as u32,
        ) >= 0;
        if state.dwm_frame {
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_BORDER_COLOR as u32,
                (&raw const palette.border).cast(),
                size_of::<COLORREF>() as u32,
            );
        }

        state.shown = true;
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        InvalidateRect(hwnd, null(), TRUE);
    }
}

unsafe fn hide(hwnd: HWND, state: &mut State) {
    unsafe {
        KillTimer(hwnd, SHOW_TIMER);
        KillTimer(hwnd, WATCH_TIMER);
        if state.shown {
            state.shown = false;
            ShowWindow(hwnd, SW_HIDE);
        }
    }
}

fn cursor_within(rect: &RECT) -> bool {
    let mut cursor = POINT { x: 0, y: 0 };
    let found = unsafe { GetCursorPos(&mut cursor) } != 0;
    found
        && (rect.left..rect.right).contains(&cursor.x)
        && (rect.top..rect.bottom).contains(&cursor.y)
}

unsafe fn paint(hwnd: HWND, state: &State) {
    unsafe {
        let mut paint = std::mem::zeroed::<PAINTSTRUCT>();
        let dc = BeginPaint(hwnd, &mut paint);
        let mut rect = std::mem::zeroed::<RECT>();
        GetClientRect(hwnd, &mut rect);
        let palette = if state.dark { &DARK } else { &LIGHT };

        let background = CreateSolidBrush(palette.background);
        FillRect(dc, &rect, background);
        DeleteObject(background);
        if !state.dwm_frame {
            let border = CreateSolidBrush(palette.border);
            FrameRect(dc, &rect, border);
            DeleteObject(border);
        }

        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, palette.text);
        let previous = SelectObject(dc, state.font);
        let mut text = RECT {
            left: rect.left + state.padding.0,
            top: rect.top + state.padding.1,
            right: rect.right - state.padding.0,
            bottom: rect.bottom - state.padding.1,
        };
        DrawTextW(
            dc,
            state.text.as_ptr(),
            state.text.len() as i32,
            &mut text,
            DT_NOPREFIX,
        );
        SelectObject(dc, previous);
        EndPaint(hwnd, &paint);
    }
}
