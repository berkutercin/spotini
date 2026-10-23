//! The window, set up as the app does after logging in.

use std::cell::RefCell;

use slint::ComponentHandle;

use crate::{keys, library_view, menu, playback, settings};

slint::include_modules!();

thread_local! {
    static WINDOW: RefCell<Option<AppWindow>> = const { RefCell::new(None) };
}

/// Runs `f` with the window. Everything in the demo is on the one browser thread.
pub fn with_window(f: impl FnOnce(&AppWindow) + 'static) {
    if let Some(window) = window() {
        f(&window);
    }
}

pub fn window() -> Option<AppWindow> {
    WINDOW.with_borrow(|window| window.as_ref().map(|w| w.clone_strong()))
}

pub fn run() {
    let window = AppWindow::new().expect("the window should open");
    settings::init(&window);
    library_view::init(&window);
    menu::init(&window);
    window.on_key(|key, shift| self::window().is_some_and(|w| keys::handle(&w, &key, shift)));
    playback::start_demo();
    window.set_title_bar_visible(false);
    window.set_username("demo".into());
    window.set_screen(Screen::Main);
    WINDOW.with_borrow_mut(|w| *w = Some(window.clone_strong()));
    // Once the event loop runs: the library loads through it.
    slint::Timer::single_shot(std::time::Duration::ZERO, || with_window(library_view::start));
    window.run().expect("the demo should run");
}
