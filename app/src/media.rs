//! Windows media integration: hardware media keys and the system media
//! overlay (title, artist, cover, play/pause) through souvlaki/SMTC.

use std::ffi::c_void;
use std::sync::Mutex;
use std::time::Duration;

use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig, SeekDirection,
};

use crate::{core, playback, ui};

/// Media controls, if the OS integration could be set up.
static CONTROLS: Mutex<Option<MediaControls>> = Mutex::new(None);

pub struct TrackMetadata<'a> {
    pub title: &'a str,
    pub artist: &'a str,
    pub album: &'a str,
    pub cover_url: Option<&'a str>,
    pub duration_ms: u32,
}

pub enum Playback {
    Playing(u32),
    Paused(u32),
    Stopped,
}

/// Registers with the OS and routes its button presses to `playback`.
///
/// Windows ties media controls to a window, so `host` is a hidden window that
/// outlives the main one: media keys keep working while the UI is closed.
pub fn init(host: *mut c_void) {
    let config = PlatformConfig {
        display_name: "spotini",
        dbus_name: "spotini",
        hwnd: Some(host),
    };
    let Ok(mut controls) = MediaControls::new(config) else { return };
    if let Err(e) = controls.attach(handle_event) {
        log::warn!("media controls unavailable: {e}");
        return;
    }
    *CONTROLS.lock().unwrap_or_else(|e| e.into_inner()) = Some(controls);
}

fn handle_event(event: MediaControlEvent) {
    let control = match event {
        MediaControlEvent::Play => "play",
        MediaControlEvent::Pause => "pause",
        MediaControlEvent::Toggle => "toggle",
        MediaControlEvent::Next => "next",
        MediaControlEvent::Previous => "previous",
        MediaControlEvent::Stop => "pause",
        MediaControlEvent::Seek(SeekDirection::Forward) | MediaControlEvent::SeekBy(SeekDirection::Forward, _) => {
            "seek-forward"
        }
        MediaControlEvent::Seek(SeekDirection::Backward) | MediaControlEvent::SeekBy(SeekDirection::Backward, _) => {
            "seek-backward"
        }
        MediaControlEvent::SetPosition(MediaPosition(position)) => {
            core::spawn(async move {
                let _ = playback::seek(position.as_millis() as u32).await;
            });
            return;
        }
        MediaControlEvent::Raise => {
            ui::show_main_window();
            return;
        }
        MediaControlEvent::Quit => {
            ui::quit();
            return;
        }
        MediaControlEvent::SetVolume(_) | MediaControlEvent::OpenUri(_) => return,
    };
    playback::control(control);
}

fn with_controls(f: impl FnOnce(&mut MediaControls)) {
    if let Some(controls) = CONTROLS.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        f(controls);
    }
}

pub fn set_metadata(track: TrackMetadata) {
    with_controls(|controls| {
        let _ = controls.set_metadata(MediaMetadata {
            title: Some(track.title),
            artist: Some(track.artist),
            album: Some(track.album),
            cover_url: track.cover_url,
            duration: Some(Duration::from_millis(track.duration_ms.into())),
        });
    });
}

pub fn set_playback(playback: Playback) {
    let progress = |ms: u32| Some(MediaPosition(Duration::from_millis(ms.into())));
    with_controls(|controls| {
        let _ = controls.set_playback(match playback {
            Playback::Playing(ms) => MediaPlayback::Playing { progress: progress(ms) },
            Playback::Paused(ms) => MediaPlayback::Paused { progress: progress(ms) },
            Playback::Stopped => MediaPlayback::Stopped,
        });
    });
}
