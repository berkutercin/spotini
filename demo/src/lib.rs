//! The website's live demo: spotini's real screens in a browser, with a made-up
//! library and no sound. The screens are the app's own code, included as is;
//! this crate swaps in what's underneath them: the library, playback and lyrics.

// The shared files and shapes have parts only the app uses.
#![allow(dead_code)]

#[path = "../../app/src/keys.rs"]
mod keys;
#[path = "../../app/src/library_view.rs"]
mod library_view;
#[path = "../../app/src/menu.rs"]
mod menu;
#[path = "../../app/src/queue.rs"]
mod queue;
#[path = "../../app/src/settings.rs"]
mod settings;
#[path = "../../app/src/side_view.rs"]
mod side_view;

mod core;
mod data;
mod engine;
mod library;
mod lyrics;
mod playback;
mod ui;

use slint::ComponentHandle;
use wasm_bindgen::prelude::wasm_bindgen;

#[wasm_bindgen(start)]
pub fn main() {
    console_error_panic_hook::set_once();
    ui::run();
}

/// The website's theme buttons: a palette ("amber", …) and a pane style ("minimal", "boxed", "glass").
#[wasm_bindgen]
pub fn set_theme(palette: &str, panes: &str) {
    let palette = palette.to_string();
    let panes = match panes {
        "boxed" => settings::Panes::Boxed,
        "glass" => settings::Panes::Glass,
        _ => settings::Panes::Minimal,
    };
    if let Some(window) = ui::window() {
        settings::update(&window, |s| {
            s.palette = palette;
            s.pane_style = panes;
        });
    }
}

/// Sizes the window to the page, in CSS pixels.
#[wasm_bindgen]
pub fn fit(width: f32, height: f32) {
    if let Some(window) = ui::window() {
        window.window().set_size(slint::LogicalSize::new(width, height));
    }
}
