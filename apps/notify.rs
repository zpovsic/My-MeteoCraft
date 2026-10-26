//! Windows notifications ("toasts") for the weather alerts.

use std::io;
use std::path::PathBuf;
use std::sync::OnceLock;

use tauri_winrt_notification::Toast;
use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegSetKeyValueW};
use windows_sys::core::PCWSTR;
use windows_sys::w;

use crate::sky::{self, Cover, Sky};

/// Identifies the app to Windows. Registering it gives the notifications the app's name
/// and icon; an app that isn't installed has no other way to provide them.
const APP_ID: &str = "MeteoCraft";
const APP_ID_KEY: PCWSTR = w!(r"Software\Classes\AppUserModelId\MeteoCraft");
const ICON_PIXELS: usize = 128;

/// Shows a notification; clicking it calls `on_click`.
pub fn show(title: &str, body: &str, on_click: impl Fn() + Send + 'static) {
    static REGISTERED: OnceLock<()> = OnceLock::new();
    REGISTERED.get_or_init(|| {
        if let Err(e) = register() {
            eprintln!("could not register for notifications: {e}");
        }
    });
    let (title, body) = (title.to_owned(), body.to_owned());
    // Showing takes a moment; the window shouldn't wait for it.
    std::thread::spawn(move || {
        let toast = Toast::new(APP_ID)
            .title(&title)
            .text1(&body)
            .on_activated(move |_| {
                on_click();
                Ok(())
            });
        if let Err(e) = toast.show() {
            eprintln!("could not show a notification: {e}");
        }
    });
}

fn register() -> io::Result<()> {
    let icon = icon_path()?;
    let symbol = Sky {
        cover: Cover::PartlyCloudy,
        precipitation: None,
        thunder: false,
        showers: false,
        night: false,
        description: String::new(),
        observed: String::new(),
    };
    let rgba = sky::icon::render(&symbol, ICON_PIXELS, 4.0);
    image::save_buffer(
        &icon,
        &rgba,
        ICON_PIXELS as u32,
        ICON_PIXELS as u32,
        image::ColorType::Rgba8,
    )
    .map_err(io::Error::other)?;
    set_string(w!("DisplayName"), APP_ID)?;
    set_string(w!("IconUri"), &icon.display().to_string())
}

fn icon_path() -> io::Result<PathBuf> {
    let dir = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::other("LOCALAPPDATA is not set"))?
        .join(APP_ID);
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("notification-icon.png"))
}

fn set_string(name: PCWSTR, value: &str) -> io::Result<()> {
    let data: Vec<u16> = value.encode_utf16().chain([0]).collect();
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            APP_ID_KEY,
            name,
            REG_SZ,
            data.as_ptr().cast(),
            (data.len() * size_of::<u16>()) as u32,
        )
    };
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(status as i32))
    }
}
