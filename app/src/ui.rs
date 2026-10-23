//! The window: Slint's event loop on the main thread, the screens before and
//! after logging in, and the glue between the window and the core.

use std::cell::RefCell;

use slint::winit_030::{WinitWindowAccessor, winit};
use slint::{CloseRequestResponse, ComponentHandle};

use crate::core::{self, UiEvent, core};
use crate::{host, keys, library_view, media, settings, tray};

slint::include_modules!();

const DASHBOARD_URL: &str = "https://developer.spotify.com/dashboard";

thread_local! {
    static WINDOW: RefCell<Option<AppWindow>> = const { RefCell::new(None) };
}

/// Runs `f` on the UI thread with the window.
pub fn with_window(f: impl FnOnce(&AppWindow) + Send + 'static) {
    on_ui_thread(move || {
        WINDOW.with_borrow(|window| {
            if let Some(window) = window {
                f(window);
            }
        })
    });
}

/// The window, when called on the UI thread.
pub fn window() -> Option<AppWindow> {
    WINDOW.with_borrow(|window| window.as_ref().map(|w| w.clone_strong()))
}

/// Runs `f` on the UI thread, from any thread.
pub fn on_ui_thread(f: impl FnOnce() + Send + 'static) {
    let _ = slint::invoke_from_event_loop(f);
}

pub fn show_main_window() {
    with_window(|window| {
        let _ = window.show();
        window.window().set_minimized(false);
        window.window().with_winit_window(|w| w.focus_window());
    });
}

pub fn quit() {
    on_ui_thread(|| {
        let _ = slint::quit_event_loop();
    });
}

/// The track in the window title, which the taskbar shows.
pub fn set_window_title(now_playing: Option<&str>) {
    let now_playing = now_playing.unwrap_or_default().to_string();
    with_window(move |window| window.set_now_playing(now_playing.into()));
}

/// Paces the software renderer's frames to the screen: it presents whenever a
/// frame is ready, out of step with refreshes, which made scrolling uneven.
/// Waiting for the compositor at each turn of the event loop lines them up.
fn pace_frames() -> slint::Timer {
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(1), || unsafe {
        windows_sys::Win32::Graphics::Dwm::DwmFlush();
    });
    timer
}

pub fn run() {
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        .select()
        .expect("the window backend should start");
    let window = AppWindow::new().expect("the window should open");
    window.set_redirect_uri(core::REDIRECT_URI.into());
    wire_window(&window);
    settings::init(&window);
    library_view::init(&window);
    crate::menu::init(&window);
    window.on_key(|key, shift| self::window().is_some_and(|w| keys::handle(&w, &key, shift)));
    WINDOW.with_borrow_mut(|w| *w = Some(window.clone_strong()));

    media::init(host::create() as *mut _);
    tray::init();
    core().set_ui(|event| with_window(move |window| on_event(window, event)));

    let _pacer = pace_frames();
    let _ = window.show();
    restore_session();
    let _ = slint::run_event_loop_until_quit();

    tray::remove();
    WINDOW.with_borrow_mut(|w| w.take());
}

fn on_event(window: &AppWindow, event: UiEvent) {
    match event {
        UiEvent::Error(message) => library_view::show_error(window, message),
        UiEvent::Track(info) => library_view::on_track(window, info),
        UiEvent::Status { state, position_ms } => library_view::on_status(window, state, position_ms),
        UiEvent::Queue(queue) => library_view::on_queue(window, queue),
    }
}

fn wire_window(window: &AppWindow) {
    // Closing keeps playing from the tray.
    window.window().on_close_requested(|| CloseRequestResponse::HideWindow);
    window.on_close_window(|| {
        with_window(|window| {
            let _ = window.hide();
        })
    });
    window.on_minimize(|| with_window(|window| window.window().set_minimized(true)));
    window.on_toggle_maximized(|| {
        with_window(|window| {
            let maximized = !window.window().is_maximized();
            window.window().set_maximized(maximized);
            window.set_is_maximized(maximized);
        })
    });
    window.on_drag(|| {
        with_window(|window| {
            window.window().with_winit_window(|w| {
                let _ = w.drag_window();
            });
        })
    });
    window.on_resize(|edge| {
        use winit::window::ResizeDirection::*;
        let direction = [North, East, South, West, NorthEast, SouthEast, SouthWest, NorthWest][edge as usize % 8];
        with_window(move |window| {
            window.window().with_winit_window(|w| {
                let _ = w.drag_resize_window(direction);
            });
        })
    });
    // Snapping or double-clicking can maximize the window too.
    let weak = window.as_weak();
    window.window().on_winit_window_event(move |w, _event| {
        if let Some(window) = weak.upgrade() {
            let maximized = w.is_maximized();
            if window.get_is_maximized() != maximized {
                window.set_is_maximized(maximized);
            }
        }
        slint::winit_030::EventResult::Propagate
    });

    window.on_login(|| {
        with_window(|window| {
            window.set_error("".into());
            window.set_screen(Screen::LoggingIn);
        });
        core::spawn(async {
            match core().login().await {
                Ok(username) => after_login(username),
                Err(e) => logged_out(Some(e)),
            }
        });
    });
    window.on_cancel_login(core::cancel_login);
    window.on_logout(|| {
        core::spawn(async {
            let result = core().logout().await;
            logged_out(result.err());
        });
    });
    window.on_open_dashboard(|| host::open_url(DASHBOARD_URL));
    window.on_save_client_id(|| {
        with_window(|window| {
            let client_id = window.get_client_id().to_string();
            window.set_error("".into());
            core::spawn(async move {
                let result = core().set_client_id(&client_id).await;
                with_window(move |window| match result {
                    Ok(()) => window.set_setup_connect_step(true),
                    Err(e) => window.set_error(e.into()),
                });
            });
        })
    });
    window.on_connect_library(|| {
        with_window(|window| {
            window.set_error("".into());
            window.set_busy(true);
        });
        core::spawn(async {
            let result = core().connect_library().await;
            with_window(move |window| {
                window.set_busy(false);
                match result {
                    Ok(()) => {
                        window.set_screen(Screen::Main);
                        library_view::start(window);
                    }
                    Err(e) => window.set_error(e.into()),
                }
            });
        });
    });
    window.on_change_client_id(|| {
        with_window(|window| {
            // From the settings: back to the setup's first step with the Client ID filled in.
            if window.get_screen() == Screen::Main {
                let status = core::library_status();
                window.set_client_id(status.client_id.unwrap_or_default().into());
                window.set_error("".into());
                window.set_screen(Screen::Setup);
            }
            window.set_setup_connect_step(false);
        })
    });
}

fn restore_session() {
    core::spawn(async {
        match core().restore_session().await {
            Ok(Some(username)) => after_login(username),
            Ok(None) => logged_out(None),
            Err(e) => logged_out(Some(e)),
        }
    });
}

/// Opens the library, or the setup if it isn't connected yet.
fn after_login(username: String) {
    let status = core::library_status();
    with_window(move |window| {
        window.set_username(username.into());
        window.set_error("".into());
        if status.connected {
            window.set_screen(Screen::Main);
            library_view::start(window);
        } else {
            window.set_client_id(status.client_id.clone().unwrap_or_default().into());
            window.set_setup_connect_step(status.client_id.is_some());
            window.set_screen(Screen::Setup);
        }
    });
}

fn logged_out(error: Option<String>) {
    with_window(move |window| {
        library_view::reset(window);
        window.set_error(error.unwrap_or_default().into());
        window.set_screen(Screen::LoggedOut);
    });
}
