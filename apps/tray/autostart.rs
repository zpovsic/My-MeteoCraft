//! Starting MeteoCraft when the user signs in, through the per-user `Run` registry key.

use std::io;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows_sys::core::PCWSTR;
use windows_sys::w;

/// Added to the `Run` command so a start at sign-in goes straight to the tray.
pub const TRAY_ARG: &str = "--tray";

const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const VALUE: PCWSTR = w!("MeteoCraft");

/// Whether Windows starts this executable at sign-in.
pub fn is_enabled() -> bool {
    match (registered(), command()) {
        (Some(registered), Ok(ours)) => registered.eq_ignore_ascii_case(&ours),
        _ => false,
    }
}

pub fn set(enabled: bool) -> io::Result<()> {
    let status = if enabled {
        let data: Vec<u16> = command()?.encode_utf16().chain([0]).collect();
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                VALUE,
                REG_SZ,
                data.as_ptr().cast(),
                (data.len() * size_of::<u16>()) as u32,
            )
        }
    } else {
        match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE) } {
            ERROR_FILE_NOT_FOUND => ERROR_SUCCESS,
            status => status,
        }
    };
    check(status)
}

fn command() -> io::Result<String> {
    let exe = std::env::current_exe()?;
    Ok(format!("\"{}\" {TRAY_ARG}", exe.display()))
}

fn registered() -> Option<String> {
    let read = |data: *mut u16, size: &mut u32| unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            VALUE,
            RRF_RT_REG_SZ,
            null_mut(),
            data.cast(),
            size,
        )
    };
    let mut size = 0;
    check(read(null_mut(), &mut size)).ok()?;
    let mut data = vec![0u16; size as usize / size_of::<u16>()];
    check(read(data.as_mut_ptr(), &mut size)).ok()?;
    data.truncate(size as usize / size_of::<u16>());
    let text = String::from_utf16_lossy(&data);
    Some(text.trim_end_matches('\0').to_owned())
}

fn check(status: WIN32_ERROR) -> io::Result<()> {
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(status as i32))
    }
}
