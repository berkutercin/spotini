//! Keyboard shortcuts for the library screen, as the help panel lists them.

use crate::library_view;
use crate::menu;
use crate::playback;
use crate::settings::{self, SidePanel};
use crate::side_view;
use crate::ui::{AppWindow, Overlay, SideTab};

const PAGE: i32 = 15;

/// Handles a key; false lets it through.
pub fn handle(window: &AppWindow, key: &str, shift: bool) -> bool {
    let overlay = window.get_overlay();
    if overlay != Overlay::None {
        let closes = key == "Escape"
            || (overlay == Overlay::Settings && key == ",")
            || (overlay == Overlay::Help && key == "?");
        if closes {
            window.set_overlay(Overlay::None);
        }
        return closes;
    }

    // Esc closes the menu; any other key closes it and goes on.
    if menu::close(window) && key == "Escape" {
        return true;
    }

    let pane = window.get_focused_pane();
    match key {
        "Down" | "Up" if shift && pane == 1 => {
            let from = window.get_cursor().max(0) as usize;
            move_cursor_keeping_selection(window, pane, if key == "Down" { 1 } else { -1 });
            library_view::extend_selection(window, from, window.get_cursor().max(0) as usize);
        }
        "j" | "Down" => move_cursor(window, pane, 1),
        "k" | "Up" => move_cursor(window, pane, -1),
        "PageDown" => move_cursor(window, pane, PAGE),
        "PageUp" => move_cursor(window, pane, -PAGE),
        "g" | "Home" => move_cursor(window, pane, i32::MIN),
        "G" | "End" => move_cursor(window, pane, i32::MAX),
        "Enter" => match pane {
            0 => library_view::open_source(window, window.get_library_cursor().max(0) as usize),
            2 => {
                if let Ok(index) = usize::try_from(window.get_queue_cursor()) {
                    side_view::jump(window, index);
                }
            }
            _ => {
                if let Ok(row) = usize::try_from(window.get_cursor()) {
                    library_view::activate(window, row);
                }
            }
        },
        "Tab" => {
            // Library, tracks, and the queue when it's showing.
            let panes = if settings::get().side_panel == SidePanel::Queue { 3 } else { 2 };
            let step = if shift { panes - 1 } else { 1 };
            window.set_focused_pane((pane.clamp(0, panes - 1) + step) % panes);
        }
        "/" => window.invoke_focus_search(),
        "Escape" if pane == 1 && library_view::clear_selection(window) => {}
        "Escape" | "Backspace" => library_view::back(window),
        "e" if pane == 1 => library_view::play_selected_next(window),
        "Delete" if pane == 2 => {
            if let Ok(index) = usize::try_from(window.get_queue_cursor()) {
                side_view::remove(window, index);
            }
        }
        "a" | "A" => {
            let Some(row) = selected_row(window) else { return true };
            if key == "a" {
                library_view::open_album_at(window, row);
            } else {
                library_view::open_artist_at(window, row);
            }
        }
        "l" => side_view::toggle_panel(window, SideTab::Lyrics),
        "q" => side_view::toggle_panel(window, SideTab::Queue),
        "?" => window.set_overlay(Overlay::Help),
        "," => window.set_overlay(Overlay::Settings),
        "o" => library_view::go_to_playing(window),
        "R" => {
            let track = selected_row(window).and_then(library_view::track_at);
            library_view::start_radio(window, track);
        }
        "s" => playback::toggle_shuffle(),
        "r" => playback::cycle_repeat(),
        "f" => library_view::toggle_like(window),
        " " => playback::control("toggle"),
        "n" => playback::control("next"),
        "p" => playback::control("previous"),
        "Right" => library_view::seek_by(window, 5_000),
        "Left" => library_view::seek_by(window, -5_000),
        "+" | "=" => library_view::change_volume_by(window, 0.05),
        "-" => library_view::change_volume_by(window, -0.05),
        _ => return false,
    }
    true
}

/// The main pane's cursor row, while the main pane has focus.
fn selected_row(window: &AppWindow) -> Option<usize> {
    if window.get_focused_pane() != 1 {
        return None;
    }
    usize::try_from(window.get_cursor()).ok()
}

fn move_cursor(window: &AppWindow, pane: i32, delta: i32) {
    move_cursor_keeping_selection(window, pane, delta);
    if pane == 1 {
        library_view::clear_selection(window);
    }
}

fn move_cursor_keeping_selection(window: &AppWindow, pane: i32, delta: i32) {
    let (cursor, len) = match pane {
        0 => (window.get_library_cursor(), library_view::sources_len()),
        2 => (window.get_queue_cursor(), side_view::upcoming_len()),
        _ => (window.get_cursor(), library_view::rows_len()),
    };
    if len == 0 {
        return;
    }
    let last = len as i64 - 1;
    let target = match delta {
        i32::MIN => 0,
        i32::MAX => last,
        _ => (cursor.max(0) as i64 + if cursor < 0 && delta > 0 { 0 } else { delta as i64 }).clamp(0, last),
    } as i32;
    match pane {
        0 => window.set_library_cursor(target),
        2 => window.set_queue_cursor(target),
        _ => window.set_cursor(skip_heading(target, delta)),
    }
}

/// Headings can't hold the cursor; step past them in the direction of travel.
fn skip_heading(row: i32, delta: i32) -> i32 {
    let len = library_view::rows_len() as i32;
    let is_heading = |r: i32| library_view::is_heading(r as usize);
    let step = if delta < 0 && delta != i32::MIN { -1 } else { 1 };
    let mut r = row;
    while (0..len).contains(&r) && is_heading(r) {
        r += step;
    }
    if (0..len).contains(&r) {
        return r;
    }
    // Ran off the end: go the other way instead.
    let mut r = row;
    while (0..len).contains(&r) && is_heading(r) {
        r -= step;
    }
    r.clamp(0, len - 1)
}
