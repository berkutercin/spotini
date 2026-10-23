//! The mini player: previous, play/pause, next and volume, sitting in the
//! taskbar just left of the notification area (above it on a vertical taskbar).
//! It's a plain Win32 window on its own thread, without a WebView, so it costs
//! a few MB and keeps working while the main window is closed.
//!
//! The buttons live in a child window of the taskbar, and the task buttons are
//! squeezed to make room for them. A hidden window of our own owns the timer
//! that keeps the two in place, and puts the buttons back after explorer restarts.
//!
//! It appears once a queue is started and stays until spotini quits or it's
//! hidden from its menu (until the next queue).

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows_sys::Win32::UI::Controls::*;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::{core, playback, ui};

/// Layout in 96-DPI pixels: square buttons, small enough for small taskbar icons.
const BUTTON: i32 = 28;
const GLYPH_SIZE: i32 = 14;
/// Space between the buttons and the notification area.
const GAP: i32 = 4;
const VOLUME_STEP: f64 = 0.05;
/// How often the buttons are put back in place after the taskbar moves things.
const TICK_TIMER: usize = 1;
const TICK_MS: u32 = 500;

/// Opacity of the hover and press backgrounds, and of the side glyphs at rest.
const HOVER_ALPHA: f32 = 0.10;
const PRESS_ALPHA: f32 = 0.06;
const SIDE_GLYPH_ALPHA: f32 = 0.8;
/// Fully clear pixels would let clicks through to the taskbar.
const CLEAR_ALPHA: f32 = 1.0 / 255.0;

const WM_APP_UPDATE: u32 = WM_APP + 1;
const WM_APP_SHUTDOWN: u32 = WM_APP + 2;
const MENU_OPEN: usize = 1;
const MENU_HIDE: usize = 2;

/// Segoe MDL2 Assets glyphs.
const GLYPH_PREVIOUS: char = '\u{E892}';
const GLYPH_NEXT: char = '\u{E893}';
const GLYPH_PLAY: char = '\u{E768}';
const GLYPH_PAUSE: char = '\u{E769}';
const GLYPH_MUTE: char = '\u{E74F}';
const GLYPH_VOLUME: [char; 3] = ['\u{E993}', '\u{E994}', '\u{E995}'];

/// Remembered between launches.
#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
struct Prefs {
    enabled: bool,
}

struct Shared {
    prefs: Prefs,
    /// A queue was started in this run.
    started: bool,
    /// Hidden from its menu until the next queue.
    hidden: bool,
    playing: bool,
    track: Option<String>,
    volume: f64,
    /// Volume to restore when unmuting.
    unmute_to: f64,
}

static SHARED: Mutex<Shared> = Mutex::new(Shared {
    prefs: Prefs { enabled: false },
    started: false,
    hidden: false,
    playing: false,
    track: None,
    volume: 0.7,
    unmute_to: 0.5,
});
/// Set once the mini player's thread runs.
static STARTED: OnceLock<()> = OnceLock::new();
/// The hidden window that runs the mini player.
static CONTROLLER: AtomicIsize = AtomicIsize::new(0);

fn with_shared<T>(f: impl FnOnce(&mut Shared) -> T) -> T {
    f(&mut SHARED.lock().unwrap_or_else(|e| e.into_inner()))
}

fn prefs_path() -> PathBuf {
    core::core().config_dir.join("mini_player.json")
}

fn save_prefs() {
    let path = prefs_path();
    let json = with_shared(|s| serde_json::to_string(&s.prefs));
    if let (Some(dir), Ok(json)) = (path.parent(), json) {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(path, json);
    }
}

/// Asks the mini player to redraw and recheck whether it should show.
fn refresh() {
    let controller = CONTROLLER.load(Ordering::Acquire);
    if controller != 0 {
        unsafe { PostMessageW(controller as HWND, WM_APP_UPDATE, 0, 0) };
    }
}

/// Loads the preferences and starts the mini player's thread (hidden until a queue starts).
pub fn init() {
    let prefs: Prefs = std::fs::read_to_string(prefs_path())
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default();
    let volume = playback::volume();
    with_shared(|s| {
        s.prefs = prefs;
        s.volume = volume;
    });
    if STARTED.set(()).is_err() {
        return;
    }
    std::thread::Builder::new()
        .name("mini-player".into())
        .spawn(run)
        .map_err(|e| log::warn!("mini player unavailable: {e}"))
        .ok();
}

/// Gives the task buttons their room back before spotini exits.
pub fn shutdown() {
    let controller = CONTROLLER.load(Ordering::Acquire);
    if controller != 0 {
        let mut result = 0;
        unsafe {
            SendMessageTimeoutW(controller as HWND, WM_APP_SHUTDOWN, 0, 0, SMTO_ABORTIFHUNG, 1000, &mut result)
        };
    }
}

pub fn enabled() -> bool {
    with_shared(|s| s.prefs.enabled)
}

/// Turning it on also brings it back if it was hidden from its menu.
pub fn set_enabled(enabled: bool) {
    with_shared(|s| {
        s.prefs.enabled = enabled;
        if enabled {
            s.hidden = false;
        }
    });
    save_prefs();
    refresh();
}

/// A new queue started: show the buttons (again, if they were hidden from the menu).
pub fn on_queue_started() {
    with_shared(|s| {
        s.started = true;
        s.hidden = false;
    });
    refresh();
}

pub fn set_track(track: Option<String>) {
    with_shared(|s| s.track = track);
    refresh();
}

pub fn set_playing(playing: bool) {
    with_shared(|s| s.playing = playing);
    refresh();
}

pub fn set_volume(volume: f64) {
    with_shared(|s| s.volume = volume);
    refresh();
}

// ---------- The windows (everything below runs on the mini player's thread) ----------

struct WindowState {
    controller: HWND,
    /// The buttons, a child of the taskbar; null until shown, and again after explorer restarts.
    buttons: HWND,
    tooltip: HWND,
    dpi: u32,
    font: HFONT,
    vertical: bool,
    /// The taskbar is light, so the glyphs are dark.
    light: bool,
    /// The task buttons were squeezed and need their room back when hiding.
    squeezed: bool,
    hovered: Option<usize>,
    pressed: Option<usize>,
    tracking_leave: bool,
}

thread_local! {
    static STATE: RefCell<WindowState> = const {
        RefCell::new(WindowState {
            controller: std::ptr::null_mut(),
            buttons: std::ptr::null_mut(),
            tooltip: std::ptr::null_mut(),
            dpi: 96,
            font: std::ptr::null_mut(),
            vertical: false,
            light: false,
            squeezed: false,
            hovered: None,
            pressed: None,
            tracking_leave: false,
        })
    };
}

/// The parts of the primary taskbar, with rectangles in its client coordinates.
struct Taskbar {
    hwnd: HWND,
    /// Holds the task buttons.
    rebar: HWND,
    rebar_rect: RECT,
    tray_rect: RECT,
    client: RECT,
}

impl Taskbar {
    unsafe fn find() -> Option<Self> {
        unsafe {
            let hwnd = FindWindowW(wide("Shell_TrayWnd").as_ptr(), std::ptr::null());
            if hwnd.is_null() {
                return None;
            }
            let rebar = FindWindowExW(hwnd, std::ptr::null_mut(), wide("ReBarWindow32").as_ptr(), std::ptr::null());
            let tray = FindWindowExW(hwnd, std::ptr::null_mut(), wide("TrayNotifyWnd").as_ptr(), std::ptr::null());
            if rebar.is_null() || tray.is_null() {
                return None;
            }
            let mut client = empty_rect();
            GetClientRect(hwnd, &mut client);
            Some(Self { hwnd, rebar, rebar_rect: rect_in(hwnd, rebar), tray_rect: rect_in(hwnd, tray), client })
        }
    }

    fn vertical(&self) -> bool {
        self.client.bottom - self.client.top > self.client.right - self.client.left
    }

    /// Where the buttons go: next to the notification area, centred across the taskbar.
    fn buttons_rect(&self, dpi: u32) -> RECT {
        let (long, short, gap) = (scale(BUTTON * 4, dpi), scale(BUTTON, dpi), scale(GAP, dpi));
        if self.vertical() {
            let left = (self.client.left + self.client.right - short) / 2;
            let bottom = self.tray_rect.top - gap;
            RECT { left, top: bottom - long, right: left + short, bottom }
        } else {
            let top = (self.client.top + self.client.bottom - short) / 2;
            let right = self.tray_rect.left - gap;
            RECT { left: right - long, top, right, bottom: top + short }
        }
    }

    /// Sets the far end of the task buttons (right edge, or bottom when vertical).
    unsafe fn set_rebar_end(&self, end: i32) {
        let r = self.rebar_rect;
        let current = if self.vertical() { r.bottom } else { r.right };
        if current == end {
            return;
        }
        let (width, height) = if self.vertical() { (r.right - r.left, end - r.top) } else { (end - r.left, r.bottom - r.top) };
        unsafe { SetWindowPos(self.rebar, std::ptr::null_mut(), r.left, r.top, width, height, SWP_NOZORDER | SWP_NOACTIVATE) };
    }

    /// Squeezes the task buttons so they end a gap before `buttons`.
    unsafe fn make_room(&self, buttons: &RECT, dpi: u32) {
        let gap = scale(GAP, dpi);
        unsafe { self.set_rebar_end(if self.vertical() { buttons.top - gap } else { buttons.left - gap }) };
    }

    /// Lets the task buttons reach the notification area again.
    unsafe fn give_room_back(&self) {
        unsafe { self.set_rebar_end(if self.vertical() { self.tray_rect.top } else { self.tray_rect.left }) };
    }
}

fn empty_rect() -> RECT {
    RECT { left: 0, top: 0, right: 0, bottom: 0 }
}

/// `child`'s rectangle in `parent`'s client coordinates.
unsafe fn rect_in(parent: HWND, child: HWND) -> RECT {
    let mut rect = empty_rect();
    unsafe {
        GetWindowRect(child, &mut rect);
        MapWindowPoints(std::ptr::null_mut(), parent, &mut rect as *mut RECT as *mut POINT, 2);
    }
    rect
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn scale(value: i32, dpi: u32) -> i32 {
    value * dpi as i32 / 96
}

/// Which button is at `offset` pixels along the bar.
fn button_at(offset: i32, dpi: u32) -> Option<usize> {
    let index = offset.div_euclid(scale(BUTTON, dpi));
    (0..4).contains(&index).then_some(index as usize)
}

/// Whether the taskbar uses the light theme.
fn taskbar_is_light() -> bool {
    let mut value: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let name = wide("SystemUsesLightTheme");
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            &mut value as *mut u32 as *mut _,
            &mut size,
        )
    };
    status == 0 && value != 0
}

fn run() {
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let controller_class = wide("spotini-mini-player-controller");
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(controller_proc),
            hInstance: instance,
            lpszClassName: controller_class.as_ptr(),
            ..std::mem::zeroed()
        });
        let buttons_class = wide("spotini-mini-player");
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(buttons_proc),
            hInstance: instance,
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            lpszClassName: buttons_class.as_ptr(),
            ..std::mem::zeroed()
        });
        // A real (never shown) top-level window, so it can own the menu and tooltip.
        let controller = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            controller_class.as_ptr(),
            wide("spotini mini player").as_ptr(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if controller.is_null() {
            log::warn!("mini player window could not be created");
            return;
        }
        let tooltip = create_tooltip(controller);
        STATE.with_borrow_mut(|st| {
            st.controller = controller;
            st.tooltip = tooltip;
            st.light = taskbar_is_light();
        });
        SetTimer(controller, TICK_TIMER, TICK_MS, None);
        CONTROLLER.store(controller as isize, Ordering::Release);
        update();

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe fn create_font(dpi: u32) -> HFONT {
    let face = wide("Segoe MDL2 Assets");
    unsafe {
        CreateFontW(
            -scale(GLYPH_SIZE, dpi),
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            // Grey antialiasing: the glyph coverage becomes the window's alpha.
            ANTIALIASED_QUALITY as u32,
            DEFAULT_PITCH as u32,
            face.as_ptr(),
        )
    }
}

unsafe fn create_tooltip(owner: HWND) -> HWND {
    unsafe {
        let class = wide("tooltips_class32");
        CreateWindowExW(
            WS_EX_TOPMOST,
            class.as_ptr(),
            std::ptr::null(),
            WS_POPUP | TTS_NOPREFIX | TTS_ALWAYSTIP,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            owner,
            std::ptr::null_mut(),
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        )
    }
}

fn tool_info(buttons: HWND, text: &mut [u16]) -> TTTOOLINFOW {
    TTTOOLINFOW {
        cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
        uFlags: TTF_IDISHWND | TTF_SUBCLASS,
        hwnd: buttons,
        uId: buttons as usize,
        lpszText: text.as_mut_ptr(),
        ..unsafe { std::mem::zeroed() }
    }
}

/// Creates the buttons inside the taskbar.
unsafe fn create_buttons(taskbar: HWND) -> HWND {
    unsafe {
        let class = wide("spotini-mini-player");
        let buttons = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_NOACTIVATE,
            class.as_ptr(),
            std::ptr::null(),
            WS_CHILD | WS_CLIPSIBLINGS,
            0,
            0,
            0,
            0,
            taskbar,
            std::ptr::null_mut(),
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        );
        if buttons.is_null() {
            log::warn!("mini player could not join the taskbar");
            return buttons;
        }
        let tooltip = STATE.with_borrow(|st| st.tooltip);
        if !tooltip.is_null() {
            let mut text = wide("spotini");
            let info = tool_info(buttons, &mut text);
            SendMessageW(tooltip, TTM_ADDTOOLW, 0, &info as *const _ as LPARAM);
            SendMessageW(tooltip, TTM_SETMAXTIPWIDTH, 0, 400);
        }
        buttons
    }
}

fn set_tooltip_text(buttons: HWND, tooltip: HWND, text: &str) {
    if tooltip.is_null() || buttons.is_null() {
        return;
    }
    let mut text = wide(text);
    let info = tool_info(buttons, &mut text);
    unsafe { SendMessageW(tooltip, TTM_UPDATETIPTEXTW, 0, &info as *const _ as LPARAM) };
}

/// Puts the buttons in place (or away) to match the shared state and the taskbar.
unsafe fn update() {
    let (should_show, tip) = with_shared(|s| {
        let volume = format!("volume {}%", (s.volume * 100.0).round());
        let tip = match &s.track {
            Some(track) => format!("{track}\n{volume}"),
            None => volume,
        };
        (s.prefs.enabled && s.started && !s.hidden, tip)
    });
    unsafe {
        // Explorer restarting takes the buttons (a child of its taskbar) with it.
        let mut buttons = STATE.with_borrow(|st| st.buttons);
        if !buttons.is_null() && IsWindow(buttons) == 0 {
            buttons = std::ptr::null_mut();
            STATE.with_borrow_mut(|st| {
                st.buttons = buttons;
                st.squeezed = false;
                st.hovered = None;
                st.pressed = None;
                st.tracking_leave = false;
            });
        }
        let Some(taskbar) = Taskbar::find() else { return };

        if !should_show {
            if !buttons.is_null() && IsWindowVisible(buttons) != 0 {
                ShowWindow(buttons, SW_HIDE);
            }
            if STATE.with_borrow(|st| st.squeezed) {
                taskbar.give_room_back();
                STATE.with_borrow_mut(|st| st.squeezed = false);
            }
            return;
        }

        if buttons.is_null() {
            buttons = create_buttons(taskbar.hwnd);
            if buttons.is_null() {
                return;
            }
            STATE.with_borrow_mut(|st| st.buttons = buttons);
        }
        let dpi = GetDpiForWindow(taskbar.hwnd).max(96);
        let old_font = STATE.with_borrow_mut(|st| {
            st.vertical = taskbar.vertical();
            st.squeezed = true;
            if st.dpi != dpi || st.font.is_null() {
                st.dpi = dpi;
                Some(std::mem::replace(&mut st.font, create_font(dpi)))
            } else {
                None
            }
        });
        if let Some(font) = old_font.filter(|font| !font.is_null()) {
            DeleteObject(font);
        }

        let target = taskbar.buttons_rect(dpi);
        let current = rect_in(taskbar.hwnd, buttons);
        if (current.left, current.top, current.right, current.bottom) != (target.left, target.top, target.right, target.bottom) {
            SetWindowPos(
                buttons,
                HWND_TOP,
                target.left,
                target.top,
                target.right - target.left,
                target.bottom - target.top,
                SWP_NOACTIVATE,
            );
        }
        taskbar.make_room(&target, dpi);
        render(buttons);
        if IsWindowVisible(buttons) == 0 {
            ShowWindow(buttons, SW_SHOWNA);
        }
        let tooltip = STATE.with_borrow(|st| st.tooltip);
        set_tooltip_text(buttons, tooltip, &tip);
    }
}

/// Draws the buttons with per-pixel alpha, so the taskbar shows through.
unsafe fn render(buttons: HWND) {
    let (playing, volume) = with_shared(|s| (s.playing, s.volume));
    let (dpi, font, vertical, light, hovered, pressed) =
        STATE.with_borrow(|st| (st.dpi, st.font, st.vertical, st.light, st.hovered, st.pressed));
    let cell = scale(BUTTON, dpi);
    let (width, height) = if vertical { (cell, cell * 4) } else { (cell * 4, cell) };
    let volume_glyph = if volume <= 0.0 {
        GLYPH_MUTE
    } else {
        GLYPH_VOLUME[((volume * 3.0).ceil() as usize).clamp(1, 3) - 1]
    };
    let glyphs = [GLYPH_PREVIOUS, if playing { GLYPH_PAUSE } else { GLYPH_PLAY }, GLYPH_NEXT, volume_glyph];
    let ink: u32 = if light { 0x00 } else { 0xff };

    unsafe {
        let screen = GetDC(std::ptr::null_mut());
        let dc = CreateCompatibleDC(screen);
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            },
            ..std::mem::zeroed()
        };
        let mut bits = std::ptr::null_mut();
        let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        if bitmap.is_null() || bits.is_null() {
            DeleteDC(dc);
            ReleaseDC(std::ptr::null_mut(), screen);
            return;
        }
        let old_bitmap = SelectObject(dc, bitmap);

        // White glyphs on black: each pixel's brightness is the glyph's coverage.
        let old_font = SelectObject(dc, font);
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, 0xffffff);
        for (i, glyph) in glyphs.iter().enumerate() {
            let start = cell * i as i32;
            let mut rect = if vertical {
                RECT { left: 0, top: start, right: cell, bottom: start + cell }
            } else {
                RECT { left: start, top: 0, right: start + cell, bottom: cell }
            };
            let mut text: Vec<u16> = glyph.to_string().encode_utf16().collect();
            DrawTextW(dc, text.as_mut_ptr(), text.len() as i32, &mut rect, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        }
        SelectObject(dc, old_font);
        GdiFlush();

        // Glyph and background share the ink colour; only their opacity differs.
        let pixels = std::slice::from_raw_parts_mut(bits as *mut u32, (width * height) as usize);
        for y in 0..height {
            for x in 0..width {
                let pixel = &mut pixels[(y * width + x) as usize];
                let button = ((if vertical { y } else { x }) / cell).min(3) as usize;
                let background = if pressed == Some(button) {
                    PRESS_ALPHA
                } else if hovered == Some(button) {
                    HOVER_ALPHA
                } else {
                    CLEAR_ALPHA
                };
                let strength = if button == 1 || hovered == Some(button) { 1.0 } else { SIDE_GLYPH_ALPHA };
                let glyph = ((*pixel >> 8) & 0xff) as f32 / 255.0 * strength;
                let alpha = glyph + background * (1.0 - glyph);
                let a = (alpha * 255.0).round() as u32;
                // Premultiplied, as UpdateLayeredWindow wants it.
                let c = ink * a / 255;
                *pixel = a << 24 | c << 16 | c << 8 | c;
            }
        }

        let size = SIZE { cx: width, cy: height };
        let source = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        UpdateLayeredWindow(buttons, screen, std::ptr::null(), &size, dc, &source, 0, &blend, ULW_ALPHA);

        SelectObject(dc, old_bitmap);
        DeleteObject(bitmap);
        DeleteDC(dc);
        ReleaseDC(std::ptr::null_mut(), screen);
    }
}

fn on_click(button: usize) {
    match button {
        0 => playback::control("previous"),
        1 => playback::control("toggle"),
        2 => playback::control("next"),
        _ => {
            let volume = with_shared(|s| {
                if s.volume > 0.0 {
                    s.unmute_to = s.volume;
                    0.0
                } else {
                    s.unmute_to
                }
            });
            playback::change_volume(volume);
        }
    }
}

fn on_wheel(notches: f64) {
    let volume = with_shared(|s| (s.volume + notches * VOLUME_STEP).clamp(0.0, 1.0));
    playback::change_volume((volume * 100.0).round() / 100.0);
}

unsafe fn show_menu() {
    let controller = STATE.with_borrow(|st| st.controller);
    unsafe {
        let menu = CreatePopupMenu();
        let open = wide("Open spotini");
        let hide = wide("Hide mini player");
        AppendMenuW(menu, MF_STRING, MENU_OPEN, open.as_ptr());
        AppendMenuW(menu, MF_STRING, MENU_HIDE, hide.as_ptr());
        let mut cursor = POINT { x: 0, y: 0 };
        GetCursorPos(&mut cursor);
        // Without this the menu doesn't close when clicking elsewhere.
        SetForegroundWindow(controller);
        let choice = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
            cursor.x,
            cursor.y,
            0,
            controller,
            std::ptr::null(),
        );
        DestroyMenu(menu);
        match choice as usize {
            MENU_OPEN => {
                ui::show_main_window();
            }
            MENU_HIDE => {
                with_shared(|s| s.hidden = true);
                update();
            }
            _ => {}
        }
    }
}

unsafe extern "system" fn controller_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_APP_UPDATE => {
                update();
                0
            }
            WM_TIMER if wparam == TICK_TIMER => {
                let light = taskbar_is_light();
                STATE.with_borrow_mut(|st| st.light = light);
                update();
                0
            }
            WM_APP_SHUTDOWN => {
                let (buttons, squeezed) = STATE.with_borrow_mut(|st| {
                    KillTimer(st.controller, TICK_TIMER);
                    (std::mem::replace(&mut st.buttons, std::ptr::null_mut()), std::mem::take(&mut st.squeezed))
                });
                if !buttons.is_null() {
                    DestroyWindow(buttons);
                }
                if squeezed && let Some(taskbar) = Taskbar::find() {
                    taskbar.give_room_back();
                }
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Redraws after a hover or press change.
fn set_highlight(hwnd: HWND, hovered: Option<usize>, pressed: Option<usize>) {
    let changed = STATE.with_borrow_mut(|st| {
        let changed = st.hovered != hovered || st.pressed != pressed;
        st.hovered = hovered;
        st.pressed = pressed;
        changed
    });
    if changed {
        unsafe { render(hwnd) };
    }
}

unsafe extern "system" fn buttons_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let x = (lparam & 0xffff) as i16 as i32;
    let y = ((lparam >> 16) & 0xffff) as i16 as i32;
    let (dpi, vertical) = STATE.with_borrow(|st| (st.dpi, st.vertical));
    let under_cursor = button_at(if vertical { y } else { x }, dpi);
    unsafe {
        match msg {
            WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
            WM_MOUSEMOVE => {
                if !STATE.with_borrow(|st| st.tracking_leave) {
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    TrackMouseEvent(&mut track);
                    STATE.with_borrow_mut(|st| st.tracking_leave = true);
                }
                let pressed = STATE.with_borrow(|st| st.pressed);
                set_highlight(hwnd, under_cursor, pressed.filter(|&p| Some(p) == under_cursor));
                0
            }
            WM_MOUSELEAVE => {
                STATE.with_borrow_mut(|st| st.tracking_leave = false);
                set_highlight(hwnd, None, None);
                0
            }
            WM_LBUTTONDOWN => {
                let tooltip = STATE.with_borrow(|st| st.tooltip);
                SendMessageW(tooltip, TTM_POP, 0, 0);
                set_highlight(hwnd, under_cursor, under_cursor);
                SetCapture(hwnd);
                0
            }
            WM_LBUTTONUP => {
                let pressed = STATE.with_borrow(|st| st.pressed);
                ReleaseCapture();
                set_highlight(hwnd, under_cursor, None);
                if let Some(button) = pressed.filter(|&p| Some(p) == under_cursor) {
                    on_click(button);
                }
                0
            }
            WM_CAPTURECHANGED => {
                let hovered = STATE.with_borrow(|st| st.hovered);
                set_highlight(hwnd, hovered, None);
                0
            }
            WM_MOUSEWHEEL => {
                let delta = ((wparam >> 16) & 0xffff) as i16 as f64;
                on_wheel(delta / WHEEL_DELTA as f64);
                0
            }
            WM_RBUTTONUP => {
                show_menu();
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}
