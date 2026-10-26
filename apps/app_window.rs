//! Opens a web page in a window of its own, without tabs or an address bar, using
//! Microsoft Edge's app mode.

/// Returns `false` if the page couldn't be opened that way, so the caller can fall back
/// to a browser tab.
#[cfg(windows)]
pub fn open(url: &str) -> bool {
    use std::ptr::null_mut;
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};

    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let file = wide("msedge");
    let parameters = wide(&format!("--app={url} --window-size=1280,900"));
    // ShellExecute finds msedge through its App Paths registration; values above 32 mean success.
    let result = unsafe {
        ShellExecuteW(
            null_mut(),
            null_mut(),
            file.as_ptr(),
            parameters.as_ptr(),
            null_mut(),
            SW_SHOWNORMAL,
        )
    };
    result as usize > 32
}

#[cfg(not(windows))]
pub fn open(_url: &str) -> bool {
    false
}
