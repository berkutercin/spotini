//! A hidden window of our own on the UI thread. It carries the media controls
//! (Windows ties them to a window) and lets a second launch bring the running
//! spotini forward instead of starting another one.

use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::ui;

const CLASS: &str = "spotini-host";
const MUTEX: &str = "spotini-native-single-instance";
const WM_APP_SHOW: u32 = WM_APP + 1;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Claims this session for this spotini. If one is already running, asks it to
/// show its window and returns false.
pub fn claim_single_instance() -> bool {
    unsafe {
        // Never closed: the mutex lives as long as the process.
        let mutex = CreateMutexW(std::ptr::null(), 0, wide(MUTEX).as_ptr());
        if mutex.is_null() || GetLastError() != ERROR_ALREADY_EXISTS {
            return true;
        }
        let running = FindWindowW(wide(CLASS).as_ptr(), std::ptr::null());
        if !running.is_null() {
            PostMessageW(running, WM_APP_SHOW, 0, 0);
        }
        false
    }
}

/// Creates the hidden window; call on the UI thread. Returns its handle.
pub fn create() -> HWND {
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let class = wide(CLASS);
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        });
        // A real (never shown) top-level window, so FindWindow sees it.
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class.as_ptr(),
            wide("spotini").as_ptr(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        )
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_APP_SHOW {
        ui::show_main_window();
        return 0;
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Opens `url` in the default browser.
pub fn open_url(url: &str) {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            wide("open").as_ptr(),
            wide(url).as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}
