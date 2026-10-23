// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod auth;
mod core;
mod engine;
mod host;
mod internal;
mod keys;
mod library;
mod library_view;
mod lyrics;
mod media;
mod menu;
mod mini;
mod output;
mod playback;
mod queue;
mod settings;
mod side_view;
mod tray;
mod ui;
mod web_api;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,librespot=info")).init();
    if !host::claim_single_instance() {
        return;
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("the async runtime should start");
    core::init(runtime.handle().clone());
    playback::load_prefs();
    playback::start_event_loop();
    mini::init();

    ui::run();

    mini::shutdown();
    core::core().shutdown();
    runtime.shutdown_timeout(std::time::Duration::from_secs(1));
}
