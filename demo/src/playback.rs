//! Pretend playback: the app's queue, a clock instead of audio, and the same
//! updates to the screens that the app's player sends.

use std::cell::RefCell;
use std::time::Duration;

use rand::seq::SliceRandom;
use slint::{Timer, TimerMode};

use crate::core::{Instant, spawn};
use crate::data;
use crate::engine::TrackInfo;
use crate::library::Track;
use crate::library_view;
use crate::queue::{Queue, Repeat};
use crate::ui;

const UPCOMING_SHOWN: usize = 100;
const RADIO_PREFIX: &str = "radio:";

#[derive(Clone)]
pub struct QueueInfo {
    pub current: Option<Track>,
    pub upcoming: Vec<Track>,
    pub source_id: Option<String>,
    pub source_name: Option<String>,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub volume: f64,
}

pub struct Snapshot {
    pub now_playing: Option<TrackInfo>,
    pub state: &'static str,
    pub position_ms: u32,
    pub queue: QueueInfo,
}

struct State {
    queue: Option<Queue>,
    shuffle: bool,
    repeat: Repeat,
    volume: f64,
    state: &'static str,
    position_ms: u32,
    position_at: Instant,
    /// Fires when the playing track would end.
    end: Timer,
}

impl State {
    fn position(&self) -> u32 {
        if self.state == "playing" {
            self.position_ms + self.position_at.elapsed().as_millis() as u32
        } else {
            self.position_ms
        }
    }

    fn current(&self) -> Option<&Track> {
        self.queue.as_ref().map(Queue::current)
    }

    fn queue_info(&self) -> QueueInfo {
        QueueInfo {
            current: self.current().cloned(),
            upcoming: self.queue.as_ref().map(|q| q.upcoming(UPCOMING_SHOWN)).unwrap_or_default(),
            source_id: self.queue.as_ref().map(|q| q.source_id.clone()),
            source_name: self.queue.as_ref().map(|q| q.source_name.clone()),
            shuffle: self.shuffle,
            repeat: self.repeat,
            volume: self.volume,
        }
    }

    /// Sets the clock and when the track ends.
    fn set_state(&mut self, state: &'static str, position_ms: u32) {
        self.state = state;
        self.position_ms = position_ms;
        self.position_at = Instant::now();
        let left = self.current().map(|t| t.duration_ms.saturating_sub(position_ms as u64));
        match left {
            Some(left) if state == "playing" => {
                self.end.start(TimerMode::SingleShot, Duration::from_millis(left), || {
                    spawn(async { next(true).await.ok(); })
                })
            }
            _ => self.end.stop(),
        }
    }
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State {
        queue: None,
        shuffle: false,
        repeat: Repeat::Off,
        volume: 0.7,
        state: "stopped",
        position_ms: 0,
        position_at: Instant::now(),
        end: Timer::default(),
    });
}

fn with_state<T>(f: impl FnOnce(&mut State) -> T) -> T {
    STATE.with_borrow_mut(f)
}

fn track_info(track: &Track) -> TrackInfo {
    TrackInfo {
        uri: track.uri.clone(),
        name: track.name.clone(),
        artists: track.artists.iter().map(|a| a.name.clone()).collect(),
        album: track.album.name.clone(),
        cover_url: track.image_url.clone(),
        duration_ms: track.duration_ms as u32,
    }
}

/// Tells the screens, after the current event, as the app's player does.
fn notify(f: impl FnOnce(&ui::AppWindow) + Send + 'static) {
    let _ = slint::invoke_from_event_loop(move || ui::with_window(f));
}

fn emit_queue() {
    let queue = with_state(|pb| pb.queue_info());
    notify(move |w| library_view::on_queue(w, queue));
}

fn emit_status() {
    let (state, position) = with_state(|pb| (pb.state, pb.position()));
    notify(move |w| library_view::on_status(w, state, position));
}

/// Starts the current track from the top.
fn start_track() {
    let info = with_state(|pb| {
        pb.set_state("playing", 0);
        pb.current().map(track_info)
    });
    emit_queue();
    notify(move |w| library_view::on_track(w, info));
    emit_status();
}

/// The demo opens with something already playing, part way in.
pub fn start_demo() {
    let tracks = data::liked_tracks();
    with_state(|pb| {
        pb.queue = Queue::new(tracks, 0, "liked".into(), false).map(|mut q| {
            q.source_name = "Liked Songs".into();
            q
        });
        pb.set_state("playing", 47_000);
    });
}

pub fn snapshot() -> Snapshot {
    with_state(|pb| Snapshot {
        now_playing: pb.current().map(track_info),
        state: pb.state,
        position_ms: pb.position(),
        queue: pb.queue_info(),
    })
}

pub async fn play_from(tracks: Vec<Track>, index: usize, source_id: String, source_name: String) -> Result<(), String> {
    let shuffle = with_state(|pb| pb.shuffle);
    start_queue(tracks, index, source_id, source_name, shuffle)
}

fn start_queue(tracks: Vec<Track>, index: usize, source_id: String, source_name: String, shuffle: bool) -> Result<(), String> {
    let mut queue = Queue::new(tracks, index, source_id, shuffle).ok_or("Nothing playable here")?;
    queue.source_name = source_name;
    with_state(|pb| pb.queue = Some(queue));
    start_track();
    Ok(())
}

pub async fn play_next(tracks: Vec<Track>) -> Result<(), String> {
    let queued = with_state(|pb| match pb.queue.as_mut() {
        Some(queue) => {
            queue.insert_next(tracks.clone());
            true
        }
        None => false,
    });
    if !queued {
        return start_queue(tracks, 0, "queue".into(), "queue".into(), false);
    }
    emit_queue();
    Ok(())
}

pub fn remove_upcoming(steps: usize) {
    if with_state(|pb| pb.queue.as_mut().is_some_and(|q| q.remove_upcoming(steps))) {
        emit_queue();
    }
}

/// A pretend station: the seed, then songs from across the library.
pub async fn start_radio(track: Track) -> Result<(), String> {
    let mut pool: Vec<Track> = data::all_tracks().into_iter().filter(|t| t.playable && t.uri != track.uri).collect();
    pool.shuffle(&mut rand::rng());
    pool.truncate(49);
    let name = format!("radio · {}", track.name);
    let id = format!("{RADIO_PREFIX}{}", track.uri);
    pool.insert(0, track);
    start_queue(pool, 0, id, name, false)
}

pub async fn next(auto: bool) -> Result<(), String> {
    let moved = with_state(|pb| {
        let (repeat, shuffle) = (pb.repeat, pb.shuffle);
        let moved = pb.queue.as_mut().is_some_and(|q| q.advance(repeat, shuffle, auto).is_some());
        if !moved && auto {
            pb.set_state("ended", 0);
        }
        moved
    });
    if moved {
        start_track();
    } else {
        emit_status();
    }
    Ok(())
}

pub async fn jump(steps: usize) -> Result<(), String> {
    if with_state(|pb| pb.queue.as_mut().is_some_and(|q| q.jump(steps).is_some())) {
        start_track();
    }
    Ok(())
}

async fn previous() -> Result<(), String> {
    let moved = with_state(|pb| {
        if pb.position() > 3000 {
            return false;
        }
        let repeat = pb.repeat;
        pb.queue.as_mut().is_some_and(|q| q.back(repeat).is_some())
    });
    if moved {
        start_track();
        Ok(())
    } else {
        seek(0).await
    }
}

fn set_paused(paused: bool) {
    let restart = with_state(|pb| {
        if pb.queue.is_none() {
            return false;
        }
        if !paused && matches!(pb.state, "ended" | "stopped") {
            return true;
        }
        let position = pb.position();
        pb.set_state(if paused { "paused" } else { "playing" }, position);
        false
    });
    if restart {
        start_track();
    } else {
        emit_status();
    }
}

/// Like librespot, seeking never changes play or pause.
pub async fn seek(position_ms: u32) -> Result<(), String> {
    with_state(|pb| {
        let state = pb.state;
        pb.set_state(state, position_ms);
    });
    emit_status();
    Ok(())
}

pub async fn set_volume(volume: f64) -> Result<(), String> {
    with_state(|pb| pb.volume = volume.clamp(0.0, 1.0));
    Ok(())
}

pub fn toggle_shuffle() {
    with_state(|pb| {
        pb.shuffle = !pb.shuffle;
        let shuffle = pb.shuffle;
        if let Some(queue) = pb.queue.as_mut() {
            queue.set_shuffle(shuffle);
        }
    });
    emit_queue();
}

pub fn cycle_repeat() {
    with_state(|pb| pb.repeat = pb.repeat.cycle());
    emit_queue();
}

pub fn control(control: &str) {
    match control {
        "toggle" => set_paused(with_state(|pb| pb.state == "playing")),
        "next" => spawn(async { next(false).await.ok(); }),
        "previous" => spawn(async { previous().await.ok(); }),
        _ => {}
    }
}
