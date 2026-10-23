//! The side panel: lyrics for the playing track, kept on the sung line, and
//! the queue. Runs on the UI thread, fed by the library screen.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use slint::{Model, ModelRc, VecModel};

use crate::core::{self, core};
use crate::engine::TrackInfo;
use crate::library::Track;
use crate::lyrics::{Lyrics, LyricsQuery};
use crate::playback::{self, QueueInfo};
use crate::ui::{self, AppWindow, LyricLine, QueueTrack, SideTab};
use crate::library_view;
use crate::menu::{self, Entry as MenuEntry};
use crate::settings::{self, SidePanel};

#[derive(Default)]
struct State {
    lines: Rc<VecModel<LyricLine>>,
    upcoming: Rc<VecModel<QueueTrack>>,
    /// Lyrics fetched this session, by track URI.
    cache: HashMap<String, Rc<Lyrics>>,
    track: Option<TrackInfo>,
    /// The track whose lyrics show, or are loading.
    lyrics_for: Option<String>,
    /// When each synced line starts; empty for plain lyrics.
    line_times: Vec<u64>,
    current_line: i32,
    position_ms: u32,
    /// The queue's current track, to tell when a new one starts.
    queue_current: Option<String>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn with_state<T>(f: impl FnOnce(&mut State) -> T) -> T {
    STATE.with_borrow_mut(f)
}

pub fn init(window: &AppWindow) {
    with_state(|s| {
        s.current_line = -1;
        window.set_lyric_lines(ModelRc::from(s.lines.clone()));
        window.set_upcoming(ModelRc::from(s.upcoming.clone()));
    });
    window.on_set_side_tab(|tab| {
        with_window(move |w| {
            settings::update(w, |s| s.side_panel = panel(tab));
            refresh_lyrics(w);
        })
    });
    window.on_seek_line(|line| {
        let time = with_state(|s| s.line_times.get(line as usize).copied());
        if let Some(time) = time {
            with_window(move |w| library_view::seek(w, time as u32));
        }
    });
    window.on_jump(|index| with_window(move |w| jump(w, index as usize)));
    window.on_remove_queued(|index| with_window(move |w| remove(w, index as usize)));
    window.on_queue_menu(|index| {
        with_window(move |w| {
            let index = index as usize;
            menu::show(
                w,
                vec![
                    MenuEntry::item("play now", move |w| jump(w, index)),
                    MenuEntry::item("remove from queue", move |w| remove(w, index)),
                ],
            );
        })
    });
}

fn panel(tab: SideTab) -> SidePanel {
    match tab {
        SideTab::Lyrics => SidePanel::Lyrics,
        SideTab::Queue => SidePanel::Queue,
    }
}

/// Shows a tab of the side panel, or hides the panel if that tab is already showing.
pub fn toggle_panel(window: &AppWindow, tab: SideTab) {
    let hide = settings::get().side_panel == panel(tab);
    settings::update(window, |s| s.side_panel = if hide { SidePanel::Hidden } else { panel(tab) });
    // The lyrics take no keys; neither does a hidden panel.
    if (hide || tab == SideTab::Lyrics) && window.get_focused_pane() == 2 {
        window.set_focused_pane(1);
    }
    refresh_lyrics(window);
}

/// Shows the queue and gives it the keys.
pub fn focus_queue(window: &AppWindow) {
    settings::update(window, |s| s.side_panel = SidePanel::Queue);
    window.set_focused_pane(2);
}

pub fn upcoming_len() -> usize {
    with_state(|s| s.upcoming.row_count())
}

/// Plays `upcoming[index]`.
pub fn jump(window: &AppWindow, index: usize) {
    window.set_queue_cursor(-1);
    core::spawn(async move {
        if let Err(e) = playback::jump(index + 1).await {
            core().error(e);
        }
    });
}

/// Takes `upcoming[index]` out of the queue; the cursor stays on that row.
pub fn remove(window: &AppWindow, index: usize) {
    playback::remove_upcoming(index + 1);
    let last = upcoming_len() as i32 - 2;
    if window.get_queue_cursor() > last {
        window.set_queue_cursor(last.max(-1));
    }
}

fn with_window(f: impl FnOnce(&AppWindow)) {
    if let Some(window) = ui::window() {
        f(&window);
    }
}

pub fn on_track(window: &AppWindow, track: Option<&TrackInfo>) {
    with_state(|s| s.track = track.cloned());
    refresh_lyrics(window);
}

pub fn on_position(window: &AppWindow, position_ms: u32) {
    let line = with_state(|s| {
        s.position_ms = position_ms;
        // The last line that has started.
        let line = s.line_times.partition_point(|&t| t <= position_ms as u64) as i32 - 1;
        let changed = line != s.current_line;
        s.current_line = line;
        changed.then_some(line)
    });
    if let Some(line) = line {
        window.set_current_line(line);
    }
}

pub fn on_queue(window: &AppWindow, queue: &QueueInfo) {
    let current = queue.current.as_ref().map(|t| t.uri.clone());
    if with_state(|s| std::mem::replace(&mut s.queue_current, current.clone()) != current) {
        window.set_queue_cursor(-1);
    }
    window.set_queue_now(queue.current.as_ref().map(queue_track).unwrap_or_default());
    with_state(|s| s.upcoming.set_vec(queue.upcoming.iter().map(queue_track).collect::<Vec<_>>()));
}

fn queue_track(track: &Track) -> QueueTrack {
    let total = track.duration_ms / 1000;
    QueueTrack {
        title: track.name.as_str().into(),
        artist: track.artists.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ").into(),
        time: format!("{}:{:02}", total / 60, total % 60).into(),
    }
}

/// Shows the playing track's lyrics, fetching them if needed. Only while the lyrics tab is open.
fn refresh_lyrics(window: &AppWindow) {
    if settings::get().side_panel != SidePanel::Lyrics {
        return;
    }
    let track = with_state(|s| s.track.clone());
    let Some(track) = track else {
        with_state(|s| s.lyrics_for = None);
        show_message(window, "", "nothing playing", false);
        return;
    };
    let cached = with_state(|s| {
        if s.lyrics_for.as_deref() == Some(&track.uri) {
            return None;
        }
        s.lyrics_for = Some(track.uri.clone());
        Some(s.cache.get(&track.uri).cloned())
    });
    match cached {
        // Already showing.
        None => {}
        Some(Some(lyrics)) => show(window, &track.uri, &lyrics),
        Some(None) => {
            show_message(window, &track.uri, "looking for lyrics…", false);
            core::spawn(async move {
                let result = core()
                    .lyrics(LyricsQuery {
                        track: &track.name,
                        artist: track.artists.first().map_or("", String::as_str),
                        album: &track.album,
                        duration_ms: track.duration_ms as u64,
                    })
                    .await;
                ui::with_window(move |w| {
                    let showing = with_state(|s| s.lyrics_for.as_deref() == Some(&track.uri));
                    match result {
                        Ok(lyrics) => {
                            let lyrics = Rc::new(lyrics);
                            with_state(|s| s.cache.insert(track.uri.clone(), lyrics.clone()));
                            if showing {
                                show(w, &track.uri, &lyrics);
                            }
                        }
                        Err(e) if showing => {
                            // Try again next time the track or tab comes up.
                            with_state(|s| s.lyrics_for = None);
                            show_message(w, &track.uri, &e, true);
                        }
                        Err(_) => {}
                    }
                });
            });
        }
    }
}

fn show_message(window: &AppWindow, key: &str, message: &str, error: bool) {
    with_state(|s| {
        s.lines.set_vec(Vec::new());
        s.line_times.clear();
        s.current_line = -1;
    });
    window.set_lyrics_key(key.into());
    window.set_lyrics_synced(false);
    window.set_current_line(-1);
    window.set_lyrics_message(message.into());
    window.set_lyrics_error(error);
}

fn show(window: &AppWindow, uri: &str, lyrics: &Lyrics) {
    let message = if lyrics.instrumental {
        "♪ instrumental"
    } else if lyrics.synced.is_none() && lyrics.plain.is_none() {
        "no lyrics found"
    } else {
        ""
    };
    show_message(window, uri, message, false);
    if !message.is_empty() {
        return;
    }
    let (lines, times, synced): (Vec<LyricLine>, Vec<u64>, bool) = match (&lyrics.synced, &lyrics.plain) {
        (Some(synced), _) => (
            synced.iter().map(|l| line(&l.text)).collect(),
            synced.iter().map(|l| l.time_ms).collect(),
            true,
        ),
        (None, Some(plain)) => (plain.lines().map(line).collect(), Vec::new(), false),
        (None, None) => return,
    };
    let position = with_state(|s| {
        s.lines.set_vec(lines);
        s.line_times = times;
        s.position_ms
    });
    window.set_lyrics_synced(synced);
    on_position(window, position);
}

fn line(text: &str) -> LyricLine {
    LyricLine {
        text: text.into(),
        gap: text.trim().is_empty(),
    }
}
