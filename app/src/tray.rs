//! The system tray icon: shows the window, controls playback, and toggles the
//! mini player. Its objects live on the UI thread.

use std::cell::RefCell;

use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::{mini, playback, ui};

thread_local! {
    static TRAY: RefCell<Option<TrayIcon>> = const { RefCell::new(None) };
    static MINI_ITEM: RefCell<Option<CheckMenuItem>> = const { RefCell::new(None) };
}

/// Creates the tray icon. Call on the UI thread.
pub fn init() {
    let mini_player = CheckMenuItem::with_id("mini", "Mini player", true, mini::enabled(), None);
    let menu = Menu::new();
    let items: [&dyn tray_icon::menu::IsMenuItem; 6] = [
        &MenuItem::with_id("show", "Show spotini", true, None),
        &MenuItem::with_id("toggle", "Play / Pause", true, None),
        &MenuItem::with_id("next", "Next", true, None),
        &MenuItem::with_id("previous", "Previous", true, None),
        &mini_player,
        &MenuItem::with_id("quit", "Quit", true, None),
    ];
    if let Err(e) = menu.append_items(&items) {
        log::warn!("tray menu: {e}");
    }
    MINI_ITEM.with_borrow_mut(|item| *item = Some(mini_player.clone()));

    // The handlers run on the UI thread, in the tray's window procedure.
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| match event.id.as_ref() {
        "show" => ui::show_main_window(),
        "mini" => {
            let enabled = !mini::enabled();
            mini::set_enabled(enabled);
            MINI_ITEM.with_borrow(|item| {
                if let Some(item) = item {
                    item.set_checked(enabled);
                }
            });
        }
        "quit" => ui::quit(),
        control => playback::control(control),
    }));
    TrayIconEvent::set_event_handler(Some(|event| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
        {
            ui::show_main_window();
        }
    }));

    let mut builder = TrayIconBuilder::new()
        .with_tooltip("spotini")
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false);
    // The app icon, from the executable's resources (see build.rs).
    if let Ok(icon) = Icon::from_resource(1, None) {
        builder = builder.with_icon(icon);
    }
    match builder.build() {
        Ok(tray) => TRAY.with_borrow_mut(|t| *t = Some(tray)),
        Err(e) => log::warn!("tray icon unavailable: {e}"),
    }
}

fn set_tooltip(text: String) {
    ui::on_ui_thread(move || {
        TRAY.with_borrow(|tray| {
            if let Some(tray) = tray {
                let _ = tray.set_tooltip(Some(&text));
            }
        })
    });
}

/// Puts the track in the window title (taskbar) and the tray tooltip.
pub fn show_now_playing(text: &str) {
    ui::set_window_title(Some(text));
    set_tooltip(format!("spotini\n{text}"));
}

pub fn clear_now_playing() {
    ui::set_window_title(None);
    set_tooltip("spotini".into());
}

/// Removes the icon before exit, so it doesn't linger in the tray.
pub fn remove() {
    TRAY.with_borrow_mut(|t| t.take());
}
