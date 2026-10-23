//! Playback control: the queue, play/pause/skip, and the state that the UI,
//! the tray and the OS media controls show. Everything here works without a
//! window, so music keeps going (and advancing) while spotini sits in the tray.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Instant;

use tokio::sync::mpsc;

use librespot_playback::player::PlayerEvent;
use serde::{Deserialize, Serialize};

use crate::core::{UiEvent, core, spawn};
use crate::engine::{self, TrackInfo};
use crate::library::Track;
use crate::queue::{Queue, Repeat};
use crate::{internal, media, mini, tray};

/// How long to wait for Connect to stop the player after a track ends.
const END_STOP_WAIT: std::time::Duration = std::time::Duration::from_millis(1500);

/// How many upcoming tracks the queue panel gets.
const UPCOMING_SHOWN: usize = 100;

/// Remembered between launches.
#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Prefs {
    pub volume: f64,
    pub shuffle: bool,
    pub repeat: Repeat,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            volume: 0.7,
            shuffle: false,
            repeat: Repeat::Off,
        }
    }
}

pub struct PlaybackState {
    queue: Option<Queue>,
    pub prefs: Prefs,
    now_playing: Option<TrackInfo>,
    state: &'static str,
    position_ms: u32,
    position_at: Instant,
    /// Unavailable tracks skipped in a row; stops the skipping once the whole queue failed.
    unavailable_streak: usize,
    /// A radio top-up is in flight.
    radio_loading: bool,
    /// A track ended and Connect is about to stop the player; the next track
    /// loads after that, or Connect's stop would cut it off.
    end_pending: bool,
}

impl Default for PlaybackState {
    fn default() -> Self {
        Self {
            queue: None,
            prefs: Prefs::default(),
            now_playing: None,
            state: "stopped",
            position_ms: 0,
            position_at: Instant::now(),
            unavailable_streak: 0,
            radio_loading: false,
            end_pending: false,
        }
    }
}

impl PlaybackState {
    fn position(&self) -> u32 {
        if self.state == "playing" {
            self.position_ms + self.position_at.elapsed().as_millis() as u32
        } else {
            self.position_ms
        }
    }

    fn is_playing(&self) -> bool {
        matches!(self.state, "playing" | "loading")
    }

    fn queue_info(&self) -> QueueInfo {
        QueueInfo {
            current: self.queue.as_ref().map(|q| q.current().clone()),
            upcoming: self.queue.as_ref().map(|q| q.upcoming(UPCOMING_SHOWN)).unwrap_or_default(),
            source_id: self.queue.as_ref().map(|q| q.source_id.clone()),
            source_name: self.queue.as_ref().map(|q| q.source_name.clone()),
            shuffle: self.prefs.shuffle,
            repeat: self.prefs.repeat,
            volume: self.prefs.volume,
        }
    }
}

#[derive(Clone)]
pub struct QueueInfo {
    pub current: Option<Track>,
    /// What plays next, in order (capped at `UPCOMING_SHOWN`).
    pub upcoming: Vec<Track>,
    pub source_id: Option<String>,
    pub source_name: Option<String>,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub volume: f64,
}

/// Everything the UI needs to draw the player after (re)opening the window.
pub struct Snapshot {
    pub now_playing: Option<TrackInfo>,
    pub state: &'static str,
    pub position_ms: u32,
    pub queue: QueueInfo,
}

fn with_state<T>(f: impl FnOnce(&mut PlaybackState) -> T) -> T {
    let mut guard = core().playback.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut guard)
}

fn prefs_path() -> PathBuf {
    core().config_dir.join("playback.json")
}

pub fn load_prefs() {
    let prefs = std::fs::read_to_string(prefs_path())
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default();
    with_state(|pb| pb.prefs = prefs);
}

fn save_prefs() {
    let path = prefs_path();
    let json = with_state(|pb| serde_json::to_string(&pb.prefs));
    if let (Some(dir), Ok(json)) = (path.parent(), json) {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(path, json);
    }
}

fn emit_queue() {
    let info = with_state(|pb| pb.queue_info());
    core().notify(UiEvent::Queue(info));
}

fn set_status(state: &'static str, position_ms: u32) {
    with_state(|pb| {
        pb.state = state;
        pb.position_ms = position_ms;
        pb.position_at = Instant::now();
    });
    mini::set_playing(matches!(state, "playing" | "loading"));
    core().notify(UiEvent::Status { state, position_ms });
}

pub fn snapshot() -> Snapshot {
    with_state(|pb| Snapshot {
        now_playing: pb.now_playing.clone(),
        state: pb.state,
        position_ms: pb.position(),
        queue: pb.queue_info(),
    })
}

pub fn volume() -> f64 {
    with_state(|pb| pb.prefs.volume)
}

async fn load(uri: &str) -> Result<(), String> {
    with_state(|pb| pb.end_pending = false);
    core().with_engine(|engine| engine.load(uri)).await
}

/// Starts a new queue from a list, at `index`.
pub async fn play_from(tracks: Vec<Track>,
    index: usize,
    source_id: String,
    source_name: String,
) -> Result<(), String> {
    let shuffle = with_state(|pb| pb.prefs.shuffle);
    start_queue(tracks, index, source_id, source_name, shuffle).await
}

async fn start_queue(
    tracks: Vec<Track>,
    index: usize,
    source_id: String,
    source_name: String,
    shuffle: bool,
) -> Result<(), String> {
    let uri = with_state(|pb| {
        let mut queue = Queue::new(tracks, index, source_id, shuffle).ok_or("Nothing playable here")?;
        queue.source_name = source_name;
        let uri = queue.current().uri.clone();
        pb.queue = Some(queue);
        Ok::<_, String>(uri)
    })?;
    emit_queue();
    mini::on_queue_started();
    load(&uri).await
}

/// Puts tracks right after the playing one; with nothing queued, plays them.
pub async fn play_next(tracks: Vec<Track>) -> Result<(), String> {
    let queued = with_state(|pb| match pb.queue.as_mut() {
        Some(queue) => {
            queue.insert_next(tracks.clone());
            true
        }
        None => false,
    });
    if !queued {
        return start_queue(tracks, 0, "queue".into(), "queue".into(), false).await;
    }
    emit_queue();
    Ok(())
}

/// Drops the track `steps` places after the playing one (1 = next).
pub fn remove_upcoming(steps: usize) {
    if with_state(|pb| pb.queue.as_mut().is_some_and(|q| q.remove_upcoming(steps))) {
        emit_queue();
    }
}

const RADIO_PREFIX: &str = "radio:";

/// Replaces the queue with a radio station seeded by `track`, in station order.
pub async fn start_radio(track: Track) -> Result<(), String> {
    let session = core().session().await?;
    let mut tracks = internal::radio(&session, &track.uri, &[]).await?;
    tracks.retain(|t| t.uri != track.uri);
    let name = format!("radio · {}", track.name());
    tracks.insert(0, track.clone());
    start_queue(tracks, 0, format!("{RADIO_PREFIX}{}", track.uri), name, false).await
}

/// Tops up a radio queue that's about to run out, in the background.
fn extend_radio_if_low() {
    let job = with_state(|pb| {
        let queue = pb.queue.as_ref()?;
        if !queue.source_id.starts_with(RADIO_PREFIX) || pb.radio_loading || queue.remaining() >= 5 {
            return None;
        }
        pb.radio_loading = true;
        Some((queue.source_id.clone(), queue.uris()))
    });
    let Some((source_id, played)) = job else { return };
    spawn(async move {
        // Spotify ignores the "already played" hint and repeats the same station,
        // so continue from the last queued track instead.
        let seed = played.last().cloned().unwrap_or_default();
        let more = match core().session().await {
            Ok(session) => internal::radio(&session, &seed, &played).await,
            Err(e) => Err(e),
        };
        with_state(|pb| {
            pb.radio_loading = false;
            if let (Ok(tracks), Some(queue)) = (more, pb.queue.as_mut()) {
                // Only if the radio is still what's playing.
                if queue.source_id == source_id {
                    queue.append(tracks);
                }
            }
        });
        emit_queue();
    });
}

/// Next track. `auto` is true when the current one ended on its own.
pub async fn next(auto: bool) -> Result<(), String> {
    let uri = with_state(|pb| {
        let (repeat, shuffle) = (pb.prefs.repeat, pb.prefs.shuffle);
        pb.queue.as_mut()?.advance(repeat, shuffle, auto).map(|t| t.uri.clone())
    });
    match uri {
        Some(uri) => {
            emit_queue();
            extend_radio_if_low();
            load(&uri).await
        }
        None => Ok(()),
    }
}

/// Skips ahead to the track `steps` places after the current one.
pub async fn jump(steps: usize) -> Result<(), String> {
    let uri = with_state(|pb| pb.queue.as_mut()?.jump(steps).map(|t| t.uri.clone()));
    match uri {
        Some(uri) => {
            emit_queue();
            extend_radio_if_low();
            load(&uri).await
        }
        None => Ok(()),
    }
}

/// Previous track, or back to the start when more than 3 s in.
pub async fn previous() -> Result<(), String> {
    let uri = with_state(|pb| {
        if pb.position() > 3000 {
            return None;
        }
        let repeat = pb.prefs.repeat;
        pb.queue.as_mut()?.back(repeat).map(|t| t.uri.clone())
    });
    match uri {
        Some(uri) => {
            emit_queue();
            load(&uri).await
        }
        None => seek(0).await,
    }
}

pub async fn set_paused(paused: bool) -> Result<(), String> {
    // After the queue ran out librespot has nothing to resume; replay the current track.
    let restart = with_state(|pb| {
        let finished = matches!(pb.state, "ended" | "stopped");
        pb.queue.as_ref().filter(|_| !paused && finished).map(|q| q.current().uri.clone())
    });
    if let Some(uri) = restart {
        return load(&uri).await;
    }
    core().with_engine(|engine| {
        if paused { engine.pause() } else { engine.play() }
        Ok(())
    })
    .await
}

pub async fn toggle() -> Result<(), String> {
    let playing = with_state(|pb| pb.is_playing());
    set_paused(playing).await
}

pub async fn seek(position_ms: u32) -> Result<(), String> {
    core().with_engine(|engine| {
        engine.seek(position_ms);
        Ok(())
    })
    .await
}

pub async fn seek_by(delta_ms: i64) -> Result<(), String> {
    let position = with_state(|pb| pb.position()) as i64;
    seek((position + delta_ms).max(0) as u32).await
}

pub async fn set_volume(volume: f64) -> Result<(), String> {
    let volume = volume.clamp(0.0, 1.0);
    with_state(|pb| pb.prefs.volume = volume);
    save_prefs();
    mini::set_volume(volume);
    core().with_engine(|engine| {
        engine.set_volume(volume);
        Ok(())
    })
    .await
}

/// Changes the volume from outside the window (the mini player) and tells the UI.
pub fn change_volume(volume: f64) {
    spawn(async move {
        if let Err(e) = set_volume(volume).await {
            core().error(e);
        }
        emit_queue();
    });
}

/// Forgets the queue and what was playing (on logout), keeping preferences.
pub fn reset() {
    with_state(|pb| {
        *pb = PlaybackState {
            prefs: pb.prefs.clone(),
            ..PlaybackState::default()
        }
    });
    media::set_playback(media::Playback::Stopped);
    tray::clear_now_playing();
    mini::set_track(None);
    core().notify(UiEvent::Track(None));
    set_status("stopped", 0);
    emit_queue();
}

pub fn toggle_shuffle() {
    with_state(|pb| {
        pb.prefs.shuffle = !pb.prefs.shuffle;
        let shuffle = pb.prefs.shuffle;
        if let Some(queue) = pb.queue.as_mut() {
            queue.set_shuffle(shuffle);
        }
    });
    save_prefs();
    emit_queue();
}

pub fn cycle_repeat() {
    with_state(|pb| pb.prefs.repeat = pb.prefs.repeat.cycle());
    save_prefs();
    emit_queue();
}

/// Runs a named control from media keys or the tray menu.
pub fn control(control: &str) {
    let control = control.to_string();
    spawn(async move {
        let result = match control.as_str() {
            "play" => set_paused(false).await,
            "pause" => set_paused(true).await,
            "toggle" => toggle().await,
            "next" => next(false).await,
            "previous" => previous().await,
            "seek-forward" => seek_by(10_000).await,
            "seek-backward" => seek_by(-10_000).await,
            _ => Ok(()),
        };
        if let Err(e) = result {
            core().error(e);
        }
    });
}

static EVENTS: OnceLock<mpsc::UnboundedSender<PlayerEvent>> = OnceLock::new();

/// Processes player events one at a time, in order, for the app's lifetime.
/// Engines come and go (reconnects); they all feed this one loop.
pub fn start_event_loop() {
    let (sender, mut receiver) = mpsc::unbounded_channel();
    if EVENTS.set(sender).is_err() {
        return;
    }
    spawn(async move {
        while let Some(event) = receiver.recv().await {
            on_event(event).await;
        }
    });
}

pub fn forward_event(event: PlayerEvent) {
    if let Some(sender) = EVENTS.get() {
        let _ = sender.send(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librespot_core::SpotifyUri;

    fn track() -> SpotifyUri {
        SpotifyUri::from_uri("spotify:track:4uLU6hMCjMI75M1A2tKUQC").unwrap()
    }

    #[test]
    fn seeking_while_paused_stays_paused() {
        let seeked = PlayerEvent::Seeked {
            play_request_id: 1,
            track_id: track(),
            position_ms: 42_000,
        };
        assert_eq!(next_status("paused", &seeked), Some(("paused", 42_000)));
    }

    #[test]
    fn seeking_while_playing_keeps_playing() {
        let seeked = PlayerEvent::Seeked {
            play_request_id: 1,
            track_id: track(),
            position_ms: 1_000,
        };
        assert_eq!(next_status("playing", &seeked), Some(("playing", 1_000)));
    }

    #[test]
    fn play_and_pause_events_set_the_state() {
        let playing = PlayerEvent::Playing {
            play_request_id: 1,
            track_id: track(),
            position_ms: 5,
        };
        let paused = PlayerEvent::Paused {
            play_request_id: 1,
            track_id: track(),
            position_ms: 6,
        };
        assert_eq!(next_status("paused", &playing), Some(("playing", 5)));
        assert_eq!(next_status("playing", &paused), Some(("paused", 6)));
    }

    #[test]
    fn position_reports_near_the_clock_are_ignored() {
        // Reports a few hundred ms either side of the clock: keep counting.
        assert!(!follows_report("playing", 13_566, 13_212));
        assert!(!follows_report("playing", 13_566, 14_400));
        // Well off (a stall, or the clock drifted): resync.
        assert!(follows_report("playing", 13_566, 10_000));
        assert!(follows_report("playing", 13_566, 16_000));
        // Not playing, the clock stands still; take the report.
        assert!(follows_report("paused", 13_566, 13_212));
    }
}

/// How far a position report may be from the clock before the clock follows it.
const POSITION_TOLERANCE_MS: u32 = 1_500;

/// Whether a periodic position report should move the clock. librespot reports
/// where decoding is, which runs 0.2-1.5 s ahead of what is heard depending on
/// packet sizes; following every report made the time jump back and forth.
fn follows_report(state: &str, clock_ms: u32, reported_ms: u32) -> bool {
    state != "playing" || clock_ms.abs_diff(reported_ms) > POSITION_TOLERANCE_MS
}

/// The playback state and position after `event`, given the current state.
/// `None` for events that don't change them.
fn next_status(current: &'static str, event: &PlayerEvent) -> Option<(&'static str, u32)> {
    match *event {
        PlayerEvent::Loading { position_ms, .. } => Some(("loading", position_ms)),
        PlayerEvent::Playing { position_ms, .. } => Some(("playing", position_ms)),
        // librespot sends Seeked while paused too, with no Paused after it, so
        // these only move the position and keep whatever state we're in.
        PlayerEvent::Seeked { position_ms, .. }
        | PlayerEvent::PositionChanged { position_ms, .. }
        | PlayerEvent::PositionCorrection { position_ms, .. } => Some((current, position_ms)),
        PlayerEvent::Paused { position_ms, .. } => Some(("paused", position_ms)),
        PlayerEvent::Stopped { .. } => Some(("stopped", 0)),
        PlayerEvent::EndOfTrack { .. } => Some(("ended", 0)),
        _ => None,
    }
}

/// Handles one librespot player event: keeps state, informs the UI and OS, and advances the queue.
async fn on_event(event: PlayerEvent) {
    match event {
        PlayerEvent::TrackChanged { audio_item } => {
            let info = engine::track_info(&audio_item);
            media::set_metadata(
                media::TrackMetadata {
                    title: &info.name,
                    artist: &info.artists.join(", "),
                    album: &info.album,
                    cover_url: info.cover_url.as_deref(),
                    duration_ms: info.duration_ms,
                },
            );
            let text = format!("{} · {}", info.name, info.artists.join(", "));
            tray::show_now_playing(&text);
            mini::set_track(Some(text));
            with_state(|pb| pb.now_playing = Some(info.clone()));
            core().notify(UiEvent::Track(Some(info)));
        }
        PlayerEvent::Loading { .. }
        | PlayerEvent::Playing { .. }
        | PlayerEvent::Seeked { .. }
        | PlayerEvent::PositionChanged { .. }
        | PlayerEvent::PositionCorrection { .. }
        | PlayerEvent::Paused { .. }
        | PlayerEvent::Stopped { .. }
        | PlayerEvent::EndOfTrack { .. } => {
            // Connect answers the end of its one-track list by stopping the player.
            if matches!(event, PlayerEvent::Stopped { .. }) && with_state(|pb| std::mem::take(&mut pb.end_pending)) {
                if let Err(e) = next(true).await {
                    core().error(e);
                }
                return;
            }
            let current = with_state(|pb| {
                if matches!(event, PlayerEvent::Playing { .. }) {
                    pb.unavailable_streak = 0;
                }
                pb.state
            });
            let Some((state, position_ms)) = next_status(current, &event) else { return };
            if matches!(event, PlayerEvent::PositionChanged { .. } | PlayerEvent::PositionCorrection { .. })
                && !follows_report(state, with_state(|pb| pb.position()), position_ms)
            {
                return;
            }
            if !matches!(event, PlayerEvent::PositionChanged { .. } | PlayerEvent::PositionCorrection { .. }) {
                media::set_playback(
                    match state {
                        "playing" | "loading" => media::Playback::Playing(position_ms),
                        "paused" => media::Playback::Paused(position_ms),
                        _ => media::Playback::Stopped,
                    },
                );
            }
            set_status(state, position_ms);
            if matches!(event, PlayerEvent::EndOfTrack { .. }) {
                if core().with_engine(|engine| Ok(engine.has_connect())).await.unwrap_or(false) {
                    // Wait for Connect's stop; go on anyway if it doesn't come.
                    with_state(|pb| pb.end_pending = true);
                    spawn(async {
                        tokio::time::sleep(END_STOP_WAIT).await;
                        if with_state(|pb| std::mem::take(&mut pb.end_pending))
                            && let Err(e) = next(true).await
                        {
                            core().error(e);
                        }
                    });
                } else if let Err(e) = next(true).await {
                    core().error(e);
                }
            }
        }
        PlayerEvent::Unavailable { .. } => {
            let gave_up = with_state(|pb| {
                pb.unavailable_streak += 1;
                pb.unavailable_streak >= pb.queue.as_ref().map_or(0, Queue::len)
            });
            if gave_up {
                core().error("None of these tracks are available.");
                set_status("stopped", 0);
            } else {
                core().error("This track is not available; skipping it.");
                let _ = next(false).await;
            }
        }
        PlayerEvent::TimeToPreloadNextTrack { .. } => {
            let uri = with_state(|pb| {
                let repeat = pb.prefs.repeat;
                pb.queue.as_ref()?.peek_next(repeat).map(|t| t.uri.clone())
            });
            if let Some(uri) = uri {
                let _ = core().with_engine(|engine| engine.preload(&uri)).await;
            }
        }
        _ => {}
    }
}
