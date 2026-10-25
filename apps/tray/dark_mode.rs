//! Matching the taskbar's light or dark theme.

use std::ffi::c_void;
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows_sys::w;

/// Whether the taskbar, and so the notification area, uses the dark theme.
pub fn system_is_dark() -> bool {
    let mut light = 1_u32;
    let mut size = size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&raw mut light).cast::<c_void>(),
            &mut size,
        )
    };
    status == ERROR_SUCCESS && light == 0
}

type SetPreferredAppMode = unsafe extern "system" fn(i32) -> i32;
type FlushMenuThemes = unsafe extern "system" fn();

const FORCE_DARK: i32 = 2;
const FORCE_LIGHT: i32 = 3;

/// Popup menus are light unless the process opts into dark mode. The uxtheme
/// functions for that are undocumented and exported only by ordinal (Windows 10
/// 1903 and later); where they are missing, menus keep the default look.
fn menu_theme_functions() -> Option<(SetPreferredAppMode, FlushMenuThemes)> {
    static FUNCTIONS: OnceLock<Option<(SetPreferredAppMode, FlushMenuThemes)>> = OnceLock::new();
    *FUNCTIONS.get_or_init(|| unsafe {
        let uxtheme = LoadLibraryW(w!("uxtheme.dll"));
        if uxtheme.is_null() {
            return None;
        }
        let set_mode = GetProcAddress(uxtheme, 135 as *const u8)?;
        let flush = GetProcAddress(uxtheme, 136 as *const u8)?;
        Some((
            std::mem::transmute::<unsafe extern "system" fn() -> isize, SetPreferredAppMode>(
                set_mode,
            ),
            std::mem::transmute::<unsafe extern "system" fn() -> isize, FlushMenuThemes>(flush),
        ))
    })
}

/// Themes popup menus opened from now on to match [`system_is_dark`].
pub fn theme_menus() {
    if let Some((set_mode, flush)) = menu_theme_functions() {
        let mode = if system_is_dark() {
            FORCE_DARK
        } else {
            FORCE_LIGHT
        };
        unsafe {
            set_mode(mode);
            flush();
        }
    }
}
