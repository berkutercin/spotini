//! The library screen: the playlists, the open view (a list, search results,
//! an album or an artist) and the player, kept in step with the core.
//! Everything here runs on the UI thread.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, Image, Model, ModelRc, Timer, TimerMode, VecModel};

use crate::core::{self, Instant, core};
use crate::engine::TrackInfo;
use crate::library::{AlbumSummary, Artist, ArtistSummary, NamedRef, Playlist, SearchResults, Track};
use crate::menu::{self, Entry as MenuEntry};
use crate::playback::{self, QueueInfo};
use crate::queue::Repeat;
use crate::settings;
use crate::side_view;
use crate::ui::{self, AppWindow, Overlay, RowKind, Screen, SourceRow, TrackRow};

const LIKED: &str = "liked";
const RADIO_PREFIX: &str = "radio:";
const ERROR_SHOWN_FOR: Duration = Duration::from_secs(6);

/// What the main pane shows.
#[derive(Clone, PartialEq)]
enum View {
    Source { id: String, name: String, owned: bool },
    Search { query: String },
    Album { id: String, name: String },
    Artist { id: String, name: String },
}

impl View {
    /// Identifies the view; also the queue's source ID when playing from it.
    fn key(&self) -> String {
        match self {
            View::Source { id, .. } => id.clone(),
            View::Search { query } => format!("search:{query}"),
            View::Album { id, .. } => format!("album:{id}"),
            View::Artist { id, .. } => format!("artist:{id}"),
        }
    }

    fn title(&self) -> String {
        match self {
            View::Source { name, .. } => name.clone(),
            View::Search { query } => format!("search: {query}"),
            View::Album { name, .. } => format!("album: {name}"),
            View::Artist { name, .. } => format!("artist: {name}"),
        }
    }
}

#[derive(Clone)]
enum ViewData {
    Tracks { tracks: Vec<Track>, album: Option<AlbumSummary> },
    Search { results: SearchResults, pages: u32 },
    Artist(Artist),
}

impl ViewData {
    /// The tracks a row's index points into.
    fn tracks(&self) -> &[Track] {
        match self {
            ViewData::Tracks { tracks, .. } => tracks,
            ViewData::Search { results, .. } => &results.tracks,
            ViewData::Artist(_) => &[],
        }
    }
}

/// What a row of the main pane stands for.
enum Entry {
    Heading,
    Track(usize),
    Album { id: String, name: String },
    Artist { id: String, name: String },
    More,
}

/// An open view and where its cursor was.
struct Visit {
    view: View,
    cursor: i32,
}

#[derive(Default)]
struct State {
    playlists: Vec<Playlist>,
    sources: Rc<VecModel<SourceRow>>,
    rows: Rc<VecModel<TrackRow>>,
    /// Views loaded this session, by key.
    cache: HashMap<String, ViewData>,
    /// Main-pane navigation; the last visit is showing. Library picks start it over.
    history: Vec<Visit>,
    /// The showing view's data and what each row stands for.
    data: Option<ViewData>,
    entries: Vec<Entry>,
    loading_more: bool,
    /// Rows picked with ctrl/shift; one or none means the cursor alone.
    selected: BTreeSet<usize>,
    /// Where a shift-click range starts.
    anchor: Option<usize>,
    /// Tracks to add to the playlist being named.
    new_playlist_uris: Vec<String>,
    /// After "from …" opened a view, the track to put the cursor on once it shows.
    pending_reveal: Option<String>,
    queue: Option<QueueInfo>,
    now_playing: Option<TrackInfo>,
    /// The playback state and where it was at `status_at`.
    state: &'static str,
    position_ms: u32,
    status_at: Option<Instant>,
    /// Moves the position between playback's reports while playing.
    clock: Timer,
    cover_url: Option<String>,
    /// The track the like button is for, and whether it's liked (None while unknown).
    liked_uri: Option<String>,
    liked: Option<bool>,
    /// A volume level waiting to be sent; dragging the bar sends one every 100 ms.
    pending_volume: Option<f64>,
    volume_timer: Timer,
    error_timer: Timer,
}

impl State {
    /// Liked Songs at 0, then the playlists.
    fn source(&self, index: usize) -> Option<View> {
        match index {
            0 => Some(View::Source {
                id: LIKED.into(),
                name: "Liked Songs".into(),
                owned: true,
            }),
            _ => self.playlists.get(index - 1).map(|p| View::Source {
                id: p.id.clone(),
                name: p.name.clone(),
                owned: p.owned,
            }),
        }
    }

    fn source_index(&self, id: &str) -> Option<usize> {
        if id == LIKED {
            return Some(0);
        }
        self.playlists.iter().position(|p| p.id == id).map(|i| i + 1)
    }

    fn view(&self) -> Option<&View> {
        self.history.last().map(|visit| &visit.view)
    }

    fn position(&self) -> u32 {
        match self.status_at {
            Some(at) if self.state == "playing" => self.position_ms + at.elapsed().as_millis() as u32,
            _ => self.position_ms,
        }
    }

    fn current(&self) -> Option<&Track> {
        self.queue.as_ref()?.current.as_ref()
    }

    /// The track behind a row of the main pane.
    fn track_at(&self, row: usize) -> Option<&Track> {
        match self.entries.get(row)? {
            Entry::Track(index) => self.data.as_ref()?.tracks().get(*index),
            _ => None,
        }
    }
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn with_state<T>(f: impl FnOnce(&mut State) -> T) -> T {
    STATE.with_borrow_mut(f)
}

/// Sets the screen up once, when the window opens.
pub fn init(window: &AppWindow) {
    with_state(|s| {
        window.set_sources(ModelRc::from(s.sources.clone()));
        window.set_tracks(ModelRc::from(s.rows.clone()));
    });
    side_view::init(window);

    window.on_open_source_at(|index| with_window(move |w| open_source(w, index as usize)));
    window.on_activate_row(|row| with_window(move |w| activate(w, row as usize)));
    window.on_select_row(|row, ctrl, shift| with_window(move |w| select_row(w, row as usize, ctrl, shift)));
    window.on_row_menu(|row| with_window(move |w| row_menu(w, row as usize)));
    window.on_new_playlist(|| with_window(|w| open_new_playlist(w, Vec::new())));
    window.on_create_playlist(|name| {
        let name = name.trim().to_string();
        if !name.is_empty() {
            with_window(move |w| create_playlist(w, name));
        }
    });
    window.on_open_artist(|row| with_window(move |w| open_artist_at(w, row as usize)));
    window.on_open_album(|row| with_window(move |w| open_album_at(w, row as usize)));
    window.on_back(|| with_window(back));
    window.on_search(|query| {
        let query = query.trim().to_string();
        if !query.is_empty() {
            with_window(move |w| navigate(w, View::Search { query }, false));
        }
    });
    window.on_previous(|| playback::control("previous"));
    window.on_toggle_play(|| playback::control("toggle"));
    window.on_next(|| playback::control("next"));
    window.on_toggle_shuffle(playback::toggle_shuffle);
    window.on_cycle_repeat(playback::cycle_repeat);
    window.on_toggle_like(|| with_window(toggle_like));
    window.on_radio(|| with_window(|w| start_radio(w, None)));
    window.on_go_to_playing(|| with_window(go_to_playing));
    window.on_seek(|ms| with_window(move |w| seek(w, ms.max(0) as u32)));
    window.on_set_volume(|level| set_volume(level as f64));
}

/// Runs `f` with the window, now: callbacks and timers are on the UI thread already.
fn with_window(f: impl FnOnce(&AppWindow)) {
    if let Some(window) = ui::window() {
        f(&window);
    }
}

/// Opens the library after logging in: loads the playlists and shows Liked Songs.
pub fn start(window: &AppWindow) {
    let snapshot = playback::snapshot();
    side_view::on_track(window, snapshot.now_playing.as_ref());
    with_state(|s| s.now_playing = snapshot.now_playing);
    apply_status(window, snapshot.state, snapshot.position_ms);
    apply_queue(window, snapshot.queue);
    refresh_now_playing(window);
    refresh_sources(window);
    open_source(window, 0);
    window.invoke_focus_keys();

    window.set_sources_loading(true);
    core::spawn(async {
        let result = core().playlists().await;
        ui::with_window(move |w| {
            w.set_sources_loading(false);
            match result {
                Ok(playlists) => {
                    with_state(|s| s.playlists = playlists);
                    refresh_sources(w);
                }
                Err(e) => show_error(w, e),
            }
        });
    });
}

/// Forgets the library on logout.
pub fn reset(window: &AppWindow) {
    with_state(|s| {
        s.playlists.clear();
        s.cache.clear();
        s.history.clear();
        s.data = None;
        s.entries.clear();
        s.pending_reveal = None;
        s.sources.set_vec(Vec::new());
        s.rows.set_vec(Vec::new());
        s.liked_uri = None;
        s.liked = None;
    });
    window.set_open_source(-1);
    window.set_view_title("".into());
    window.set_view_detail("".into());
    window.set_can_go_back(false);
    window.set_cursor(-1);
}

pub fn on_track(window: &AppWindow, info: Option<TrackInfo>) {
    side_view::on_track(window, info.as_ref());
    with_state(|s| s.now_playing = info);
    refresh_now_playing(window);
}

pub fn on_status(window: &AppWindow, state: &'static str, position_ms: u32) {
    apply_status(window, state, position_ms);
}

pub fn on_queue(window: &AppWindow, queue: QueueInfo) {
    apply_queue(window, queue);
    refresh_now_playing(window);
    refresh_sources(window);
}

/// Shows a playback error above the player for a few seconds.
pub fn show_error(window: &AppWindow, message: String) {
    show_toast(window, message, false);
}

/// Shows news ("added 3 tracks to …") where errors show.
fn show_notice(window: &AppWindow, message: String) {
    show_toast(window, message, true);
}

fn show_toast(window: &AppWindow, message: String, notice: bool) {
    window.set_error_is_notice(notice);
    window.set_error(message.into());
    with_state(|s| {
        s.error_timer.start(TimerMode::SingleShot, ERROR_SHOWN_FOR, || {
            with_window(|w| {
                if w.get_screen() == Screen::Main {
                    w.set_error("".into());
                }
            })
        })
    });
}

fn refresh_sources(window: &AppWindow) {
    with_state(|s| {
        let playing = s.queue.as_ref().and_then(|q| q.source_id.clone());
        let row = |id: &str, name: &str, count: Option<usize>| SourceRow {
            name: name.into(),
            count: count.map(|c| c.to_string()).unwrap_or_default().into(),
            playing: playing.as_deref() == Some(id),
        };
        let liked_count = match s.cache.get(LIKED) {
            Some(ViewData::Tracks { tracks, .. }) => Some(tracks.len()),
            _ => None,
        };
        let mut rows = vec![row(LIKED, "Liked Songs", liked_count)];
        rows.extend(
            s.playlists
                .iter()
                .map(|p| row(&p.id, &p.name, p.track_count.map(|c| c as usize))),
        );
        s.sources.set_vec(rows);
        let open = match s.view() {
            Some(View::Source { id, .. }) => s.source_index(id),
            _ => None,
        };
        window.set_open_source(open.map_or(-1, |i| i as i32));
    });
}

pub fn open_source(window: &AppWindow, index: usize) {
    if let Some(view) = with_state(|s| s.source(index)) {
        navigate(window, view, true);
    }
}

/// Opens a view. Library picks start the history over; everything else stacks on it.
fn navigate(window: &AppWindow, view: View, from_library: bool) {
    window.set_focused_pane(1);
    let cursor = window.get_cursor();
    let same = with_state(|s| {
        let same = s.view() == Some(&view);
        if same && (!from_library || s.history.len() == 1) {
            return true;
        }
        if let Some(visit) = s.history.last_mut() {
            visit.cursor = cursor;
        }
        if from_library {
            s.history.clear();
        }
        if !same || from_library {
            s.history.push(Visit { view, cursor: -1 });
        }
        false
    });
    if same {
        reveal_pending(window);
    } else {
        show_view(window);
    }
}

pub fn back(window: &AppWindow) {
    if with_state(|s| s.history.len() > 1 && s.history.pop().is_some()) {
        show_view(window);
    }
}

/// Draws the last visit: from the cache, or loading it.
fn show_view(window: &AppWindow) {
    let Some((view, cursor, depth)) = with_state(|s| {
        let visit = s.history.last()?;
        Some((visit.view.clone(), visit.cursor, s.history.len()))
    }) else {
        return;
    };
    window.set_view_title(view.title().into());
    window.set_can_go_back(depth > 1);
    window.set_tracks_error("".into());
    refresh_sources(window);
    let key = view.key();
    let cached = with_state(|s| {
        s.data = None;
        s.loading_more = false;
        s.cache.get(&key).cloned()
    });
    if let Some(data) = cached {
        render(window, data);
        window.set_cursor(cursor);
        if cursor >= 0 {
            window.invoke_reveal_track(cursor);
        }
        reveal_pending(window);
        return;
    }
    window.set_cursor(-1);
    window.set_view_detail("".into());
    window.set_empty_text("".into());
    window.set_current_index(-1);
    with_state(|s| {
        s.entries.clear();
        s.rows.set_vec(Vec::new());
    });
    window.set_tracks_loading(true);
    core::spawn(async move {
        let result = fetch(&view).await;
        ui::with_window(move |w| {
            let showing = with_state(|s| {
                if let Ok(data) = &result {
                    s.cache.insert(key.clone(), data.clone());
                }
                s.view().is_some_and(|v| v.key() == key)
            });
            refresh_sources(w);
            if !showing {
                return;
            }
            w.set_tracks_loading(false);
            match result {
                Ok(data) => {
                    render(w, data);
                    reveal_pending(w);
                }
                Err(e) => w.set_tracks_error(e.into()),
            }
        });
    });
}

async fn fetch(view: &View) -> Result<ViewData, String> {
    Ok(match view {
        View::Source { id, owned, .. } => {
            let tracks = if id == LIKED {
                core().liked_tracks().await?
            } else {
                core().playlist_tracks(id, *owned).await?
            };
            ViewData::Tracks { tracks, album: None }
        }
        View::Search { query } => ViewData::Search {
            results: core().search(query, 0).await?,
            pages: 1,
        },
        View::Album { id, .. } => {
            let album = core().album(id).await?;
            ViewData::Tracks {
                tracks: album.tracks,
                album: Some(album.summary),
            }
        }
        View::Artist { id, .. } => ViewData::Artist(core().artist(id).await?),
    })
}

/// Fills the main pane from a view's data.
fn render(window: &AppWindow, data: ViewData) {
    let mut rows = Vec::new();
    let mut entries = Vec::new();
    let (detail, empty) = match &data {
        ViewData::Tracks { tracks, album } => {
            push_tracks(&mut rows, &mut entries, tracks);
            let total_ms: u64 = tracks.iter().map(|t| t.duration_ms).sum();
            let album = album
                .as_ref()
                .map(|a| format!("{} · {} · ", artist_names(&a.artists), a.year.as_deref().unwrap_or_default()))
                .unwrap_or_default();
            let detail = format!("{album}{} tracks · {} min", tracks.len(), (total_ms as f64 / 60_000.0).round());
            (detail, "no tracks here yet")
        }
        ViewData::Search { results, .. } => {
            if !results.tracks.is_empty() {
                push_heading(&mut rows, &mut entries, "tracks");
                push_tracks(&mut rows, &mut entries, &results.tracks);
            }
            if !results.albums.is_empty() {
                push_heading(&mut rows, &mut entries, "albums");
                push_albums(&mut rows, &mut entries, &results.albums);
            }
            if !results.artists.is_empty() {
                push_heading(&mut rows, &mut entries, "artists");
                push_artists(&mut rows, &mut entries, &results.artists);
            }
            if results.has_more {
                rows.push(TrackRow {
                    kind: RowKind::More,
                    num: "+".into(),
                    title: "load 10 more of each".into(),
                    playable: true,
                    ..Default::default()
                });
                entries.push(Entry::More);
            }
            let detail = format!(
                "{} tracks · {} albums · {} artists",
                results.tracks.len(),
                results.albums.len(),
                results.artists.len()
            );
            (detail, "nothing found")
        }
        ViewData::Artist(artist) => {
            push_albums(&mut rows, &mut entries, &artist.albums);
            (format!("{} albums & singles", artist.albums.len()), "no albums")
        }
    };
    window.set_view_detail(detail.into());
    window.set_empty_text(empty.into());
    window.set_tracks_loading(false);
    with_state(|s| {
        s.rows.set_vec(rows);
        s.entries = entries;
        s.data = Some(data);
        s.selected.clear();
        s.anchor = None;
    });
    window.set_selection_count(0);
    refresh_current(window);
}

fn push_heading(rows: &mut Vec<TrackRow>, entries: &mut Vec<Entry>, title: &str) {
    rows.push(TrackRow {
        kind: RowKind::Heading,
        title: title.into(),
        playable: true,
        ..Default::default()
    });
    entries.push(Entry::Heading);
}

fn push_tracks(rows: &mut Vec<TrackRow>, entries: &mut Vec<Entry>, tracks: &[Track]) {
    for (i, track) in tracks.iter().enumerate() {
        rows.push(TrackRow {
            kind: RowKind::Track,
            num: (i + 1).to_string().into(),
            title: track.name.as_str().into(),
            artist: artist_names(&track.artists).into(),
            album: track.album.name.as_str().into(),
            time: format_time(track.duration_ms).into(),
            playable: track.playable,
            artist_link: track.artists.iter().any(|a| a.id.is_some()),
            album_link: track.album.id.is_some(),
            selected: false,
        });
        entries.push(Entry::Track(i));
    }
}

fn push_albums(rows: &mut Vec<TrackRow>, entries: &mut Vec<Entry>, albums: &[AlbumSummary]) {
    for album in albums {
        let kind = match &album.year {
            Some(year) => format!("{} · {year}", album.album_type),
            None => album.album_type.clone(),
        };
        rows.push(TrackRow {
            kind: RowKind::Album,
            num: "◆".into(),
            title: album.name.as_str().into(),
            artist: artist_names(&album.artists).into(),
            album: kind.into(),
            time: album.total_tracks.map(|n| n.to_string()).unwrap_or_default().into(),
            playable: true,
            ..Default::default()
        });
        entries.push(Entry::Album {
            id: album.id.clone(),
            name: album.name.clone(),
        });
    }
}

fn push_artists(rows: &mut Vec<TrackRow>, entries: &mut Vec<Entry>, artists: &[ArtistSummary]) {
    for artist in artists {
        rows.push(TrackRow {
            kind: RowKind::Artist,
            num: "@".into(),
            title: artist.name.as_str().into(),
            artist: "artist".into(),
            playable: true,
            ..Default::default()
        });
        entries.push(Entry::Artist {
            id: artist.id.clone(),
            name: artist.name.clone(),
        });
    }
}

fn artist_names(artists: &[NamedRef]) -> String {
    artists.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ")
}

fn format_time(ms: u64) -> String {
    let total = ms / 1000;
    format!("{}:{:02}", total / 60, total % 60)
}

/// Marks the playing track's row, if it was started from the showing view.
fn refresh_current(window: &AppWindow) {
    let row = with_state(|s| {
        let queue = s.queue.as_ref()?;
        if queue.source_id.as_deref() != s.view().map(View::key).as_deref() {
            return None;
        }
        let uri = &queue.current.as_ref()?.uri;
        (0..s.entries.len()).find(|&row| s.track_at(row).is_some_and(|t| t.uri == *uri))
    });
    window.set_current_index(row.map_or(-1, |i| i as i32));
}

/// Puts the cursor on the track "from …" asked for, once its view is showing.
fn reveal_pending(window: &AppWindow) {
    let row = with_state(|s| {
        s.data.as_ref()?;
        let uri = s.pending_reveal.take()?;
        (0..s.entries.len()).find(|&row| s.track_at(row).is_some_and(|t| t.uri == uri))
    });
    if let Some(row) = row {
        window.set_cursor(row as i32);
        window.invoke_reveal_track(row as i32);
    }
}

/// A double-click: plays a track, opens an album or artist, or loads more results.
pub fn activate(window: &AppWindow, row: usize) {
    enum Action {
        Play(Vec<Track>, usize, View),
        Open(View),
        More,
    }
    let action = with_state(|s| {
        Some(match s.entries.get(row)? {
            Entry::Heading => return None,
            Entry::Track(index) => {
                let tracks = s.data.as_ref()?.tracks().to_vec();
                Action::Play(tracks, *index, s.view()?.clone())
            }
            Entry::Album { id, name } => Action::Open(View::Album {
                id: id.clone(),
                name: name.clone(),
            }),
            Entry::Artist { id, name } => Action::Open(View::Artist {
                id: id.clone(),
                name: name.clone(),
            }),
            Entry::More => Action::More,
        })
    });
    match action {
        Some(Action::Play(tracks, index, view)) => {
            if !tracks.get(index).is_some_and(|t| t.playable) {
                return;
            }
            core::spawn(async move {
                if let Err(e) = playback::play_from(tracks, index, view.key(), view.title()).await {
                    core().error(e);
                }
            });
        }
        Some(Action::Open(view)) => navigate(window, view, false),
        Some(Action::More) => load_more(),
        None => {}
    }
}

pub fn open_artist_at(window: &AppWindow, row: usize) {
    let view = with_state(|s| {
        let artist = s.track_at(row)?.artists.iter().find(|a| a.id.is_some())?;
        Some(View::Artist {
            id: artist.id.clone()?,
            name: artist.name.clone(),
        })
    });
    if let Some(view) = view {
        navigate(window, view, false);
    }
}

pub fn open_album_at(window: &AppWindow, row: usize) {
    let view = with_state(|s| {
        let album = &s.track_at(row)?.album;
        Some(View::Album {
            id: album.id.clone()?,
            name: album.name.clone(),
        })
    });
    if let Some(view) = view {
        navigate(window, view, false);
    }
}

/// Adds the next page of search results.
fn load_more() {
    let Some((query, pages, key)) = with_state(|s| {
        let View::Search { query } = s.view()? else { return None };
        let Some(ViewData::Search { results, pages }) = &s.data else { return None };
        if !results.has_more || s.loading_more {
            return None;
        }
        let job = (query.clone(), *pages, s.view()?.key());
        s.loading_more = true;
        Some(job)
    }) else {
        return;
    };
    let more_row = with_state(|s| s.rows.row_count().checked_sub(1));
    if let Some(row) = more_row {
        set_row_title(row, "loading…");
    }
    core::spawn(async move {
        let result = core().search(&query, pages).await;
        ui::with_window(move |w| {
            let merged = with_state(|s| {
                s.loading_more = false;
                if s.view().map(View::key).as_deref() != Some(&key) {
                    return None;
                }
                let Some(ViewData::Search { results, pages }) = s.data.clone() else { return None };
                Some(result.map(|page| {
                    let mut results = results;
                    results.tracks.extend(page.tracks);
                    results.albums.extend(page.albums);
                    results.artists.extend(page.artists);
                    results.has_more = page.has_more;
                    ViewData::Search { results, pages: pages + 1 }
                }))
            });
            match merged {
                Some(Ok(data)) => {
                    with_state(|s| s.cache.insert(key, data.clone()));
                    let cursor = w.get_cursor();
                    render(w, data);
                    w.set_cursor(cursor);
                }
                Some(Err(e)) => {
                    if let Some(row) = more_row {
                        set_row_title(row, "load 10 more of each");
                    }
                    show_error(w, e);
                }
                None => {}
            }
        });
    });
}

fn set_row_title(row: usize, title: &str) {
    with_state(|s| {
        if let Some(mut data) = s.rows.row_data(row) {
            data.title = title.into();
            s.rows.set_row_data(row, data);
        }
    });
}

fn apply_status(window: &AppWindow, state: &'static str, position_ms: u32) {
    with_state(|s| {
        s.state = state;
        s.position_ms = position_ms;
        s.status_at = Some(Instant::now());
        if state == "playing" {
            s.clock.start(TimerMode::Repeated, Duration::from_millis(250), tick);
        } else {
            s.clock.stop();
        }
    });
    window.set_is_playing(matches!(state, "playing" | "loading"));
    window.set_position_ms(position_ms as i32);
    side_view::on_position(window, position_ms);
}

fn tick() {
    with_window(|w| {
        if w.window().is_visible() {
            let position = with_state(|s| s.position());
            w.set_position_ms(position as i32);
            side_view::on_position(w, position);
        }
    });
}

fn apply_queue(window: &AppWindow, queue: QueueInfo) {
    window.set_has_queue(queue.current.is_some());
    window.set_shuffle(queue.shuffle);
    window.set_repeat(
        match queue.repeat {
            Repeat::Off => "off",
            Repeat::All => "all",
            Repeat::One => "one",
        }
        .into(),
    );
    window.set_np_source(queue.source_name.clone().unwrap_or_default().into());
    window.set_radio_on(queue.source_id.as_deref().is_some_and(|id| id.starts_with(RADIO_PREFIX)));
    let dragging_volume = with_state(|s| s.pending_volume.is_some() || s.volume_timer.running());
    if !dragging_volume {
        window.set_volume(queue.volume as f32);
    }
    side_view::on_queue(window, &queue);
    with_state(|s| s.queue = Some(queue));
    refresh_current(window);
}

/// The player's track: what librespot loaded, or else the queue's current track.
fn refresh_now_playing(window: &AppWindow) {
    let (title, subtitle, duration, cover, uri) = with_state(|s| {
        if let Some(info) = &s.now_playing {
            let subtitle = format!("{} — {}", info.artists.join(", "), info.album);
            (info.name.clone(), subtitle, info.duration_ms as u64, info.cover_url.clone(), Some(info.uri.clone()))
        } else if let Some(track) = s.current() {
            let subtitle = format!("{} — {}", artist_names(&track.artists), track.album.name);
            (track.name.clone(), subtitle, track.duration_ms, track.image_url.clone(), Some(track.uri.clone()))
        } else {
            Default::default()
        }
    });
    window.set_has_track(uri.is_some());
    window.set_np_title(title.into());
    window.set_np_subtitle(subtitle.into());
    window.set_duration_ms(duration as i32);
    load_cover(window, cover);
    // Like what the queue says is playing; librespot's URI can differ for relinked tracks.
    let like_uri = with_state(|s| s.current().map(|t| t.uri.clone())).or(uri);
    check_liked(window, like_uri);
}

fn load_cover(window: &AppWindow, url: Option<String>) {
    let changed = with_state(|s| {
        let changed = s.cover_url != url;
        s.cover_url = url.clone();
        changed
    });
    if !changed {
        return;
    }
    let Some(url) = url else {
        window.set_cover(Image::default());
        settings::set_cover(window, None);
        return;
    };
    core::spawn(async move {
        let cover = core().cover(&url).await;
        ui::with_window(move |w| {
            if with_state(|s| s.cover_url.as_deref() == Some(&url))
                && let Ok((pixels, colors)) = cover
            {
                w.set_cover(Image::from_rgba8(pixels));
                settings::set_cover(w, colors);
            }
        });
    });
}

fn check_liked(window: &AppWindow, uri: Option<String>) {
    let changed = with_state(|s| {
        let changed = s.liked_uri != uri;
        if changed {
            s.liked_uri = uri.clone();
            s.liked = None;
        }
        changed
    });
    if !changed {
        return;
    }
    window.set_liked(-1);
    let Some(uri) = uri else { return };
    core::spawn(async move {
        let result = core().is_liked(uri.clone()).await;
        ui::with_window(move |w| {
            if let Ok(liked) = result {
                set_liked_state(w, &uri, liked);
            }
        });
    });
}

fn set_liked_state(window: &AppWindow, uri: &str, liked: bool) {
    if with_state(|s| {
        let current = s.liked_uri.as_deref() == Some(uri);
        if current {
            s.liked = Some(liked);
        }
        current
    }) {
        window.set_liked(liked as i32);
    }
}

pub fn toggle_like(window: &AppWindow) {
    let Some((uri, liked)) = with_state(|s| Some((s.liked_uri.clone()?, s.liked?))) else { return };
    set_liked_state(window, &uri, !liked);
    core::spawn(async move {
        let result = core().set_liked(&uri, !liked).await;
        ui::with_window(move |w| match result {
            Ok(()) => reload_list(w, LIKED),
            Err(e) => {
                set_liked_state(w, &uri, liked);
                show_error(w, e);
            }
        });
    });
}

/// A list changed: reload it next time, or now if it's showing.
fn reload_list(window: &AppWindow, key: &str) {
    let showing = with_state(|s| {
        s.cache.remove(key);
        s.view().is_some_and(|v| v.key() == key)
    });
    if showing {
        let cursor = window.get_cursor();
        with_state(|s| {
            if let Some(visit) = s.history.last_mut() {
                visit.cursor = cursor;
            }
        });
        show_view(window);
    }
    refresh_sources(window);
}

/// Starts a radio from `track`, or from what's playing.
pub fn start_radio(window: &AppWindow, track: Option<Track>) {
    let Some(track) = track.or_else(|| with_state(|s| s.current().cloned())) else { return };
    window.set_radio_loading(true);
    core::spawn(async move {
        let result = playback::start_radio(track).await;
        ui::with_window(move |w| {
            w.set_radio_loading(false);
            if let Err(e) = result {
                show_error(w, e);
            }
        });
    });
}

/// Opens the view that's playing, with the cursor on the playing track.
pub fn go_to_playing(window: &AppWindow) {
    let Some(view) = with_state(|s| {
        let queue = s.queue.as_ref()?;
        let id = queue.source_id.as_deref()?;
        let target = if let Some(index) = s.source_index(id) {
            (s.source(index)?, true)
        } else if let Some(album) = id.strip_prefix("album:") {
            let name = queue.source_name.as_deref().unwrap_or_default();
            let name = name.strip_prefix("album: ").unwrap_or(name).to_string();
            (View::Album { id: album.into(), name }, false)
        } else if let Some(query) = id.strip_prefix("search:") {
            (View::Search { query: query.into() }, false)
        } else {
            // A radio has no list of its own; its tracks are in the queue panel.
            return Some(None);
        };
        s.pending_reveal = queue.current.as_ref().map(|t| t.uri.clone());
        Some(Some(target))
    }) else {
        return;
    };
    match view {
        Some((view, from_library)) => navigate(window, view, from_library),
        None => side_view::focus_queue(window),
    }
}

pub fn seek(window: &AppWindow, position_ms: u32) {
    with_state(|s| {
        s.position_ms = position_ms;
        s.status_at = Some(Instant::now());
    });
    window.set_position_ms(position_ms as i32);
    side_view::on_position(window, position_ms);
    core::spawn(async move {
        if let Err(e) = playback::seek(position_ms).await {
            core().error(e);
        }
    });
}

fn set_volume(level: f64) {
    with_state(|s| {
        s.pending_volume = Some(level);
        if !s.volume_timer.running() {
            s.volume_timer.start(TimerMode::SingleShot, Duration::from_millis(100), send_volume);
        }
    });
}

fn send_volume() {
    let Some(level) = with_state(|s| s.pending_volume.take()) else { return };
    core::spawn(async move {
        if let Err(e) = playback::set_volume(level).await {
            core().error(e);
        }
    });
}

pub fn sources_len() -> usize {
    with_state(|s| s.playlists.len() + 1)
}

pub fn rows_len() -> usize {
    with_state(|s| s.entries.len())
}

/// The track behind a row of the main pane.
pub fn track_at(row: usize) -> Option<Track> {
    with_state(|s| s.track_at(row).cloned())
}

pub fn seek_by(window: &AppWindow, delta_ms: i64) {
    let position = with_state(|s| s.position()) as i64;
    seek(window, (position + delta_ms).max(0) as u32);
}

pub fn change_volume_by(window: &AppWindow, delta: f32) {
    let level = (window.get_volume() + delta).clamp(0.0, 1.0);
    window.set_volume(level);
    set_volume(level as f64);
}

pub fn is_heading(row: usize) -> bool {
    with_state(|s| matches!(s.entries.get(row), Some(Entry::Heading)))
}

/// Updates the selection and the rows that show it.
fn set_selection(window: &AppWindow, selected: BTreeSet<usize>) {
    let count = with_state(|s| {
        let changed: Vec<usize> = s.selected.symmetric_difference(&selected).copied().collect();
        for row in changed {
            if let Some(mut data) = s.rows.row_data(row) {
                data.selected = selected.contains(&row);
                s.rows.set_row_data(row, data);
            }
        }
        s.selected = selected;
        s.selected.len()
    });
    window.set_selection_count(count as i32);
}

/// A click on a row: picks it, or with ctrl adds or drops it, or with shift
/// picks the range from the last click.
pub fn select_row(window: &AppWindow, row: usize, ctrl: bool, shift: bool) {
    let cursor = usize::try_from(window.get_cursor()).ok();
    let selected = with_state(|s| {
        let is_track = |r: usize| matches!(s.entries.get(r), Some(Entry::Track(_)));
        if !is_track(row) {
            return None;
        }
        let mut selected = s.selected.clone();
        if shift {
            let anchor = s.anchor.or(cursor).unwrap_or(row);
            selected = (anchor.min(row)..=anchor.max(row)).filter(|&r| is_track(r)).collect();
        } else if ctrl {
            // The cursor stands for a selection of one.
            if selected.is_empty()
                && let Some(cursor) = cursor.filter(|&c| c != row && is_track(c))
            {
                selected.insert(cursor);
            }
            if !selected.remove(&row) {
                selected.insert(row);
            }
            s.anchor = Some(row);
        } else {
            selected = BTreeSet::from([row]);
            s.anchor = Some(row);
        }
        Some(selected)
    });
    if let Some(selected) = selected {
        set_selection(window, selected);
    }
}

/// Shift with the arrow keys: grows the selection from `from` to `to`.
pub fn extend_selection(window: &AppWindow, from: usize, to: usize) {
    let selected = with_state(|s| {
        let anchor = *s.anchor.get_or_insert(from);
        (anchor.min(to)..=anchor.max(to))
            .filter(|&r| matches!(s.entries.get(r), Some(Entry::Track(_))))
            .collect()
    });
    set_selection(window, selected);
}

/// Drops a multi-row selection; true if there was one.
pub fn clear_selection(window: &AppWindow) -> bool {
    let had = with_state(|s| {
        s.anchor = None;
        s.selected.len() > 1
    });
    set_selection(window, BTreeSet::new());
    had
}

/// The rows an action on `row` covers: the selection if `row` is in it, else `row`.
fn rows_for(row: usize) -> Vec<usize> {
    with_state(|s| {
        if s.selected.len() > 1 && s.selected.contains(&row) {
            s.selected.iter().copied().collect()
        } else {
            vec![row]
        }
    })
}

fn tracks_at(rows: &[usize]) -> Vec<Track> {
    with_state(|s| rows.iter().filter_map(|&r| s.track_at(r).cloned()).collect())
}

fn count(n: usize) -> String {
    if n == 1 { "1 track".into() } else { format!("{n} tracks") }
}

/// The right-click menu for a track row (and the rest of the selection).
pub fn row_menu(window: &AppWindow, row: usize) {
    if with_state(|s| s.selected.len() > 1 && !s.selected.contains(&row)) {
        clear_selection(window);
    }
    window.set_cursor(row as i32);
    let rows = rows_for(row);
    let tracks = tracks_at(&rows);
    let Some(view) = with_state(|s| s.view().cloned()) else { return };
    if tracks.is_empty() {
        return;
    }
    let playable: Vec<Track> = tracks.iter().filter(|t| t.playable).cloned().collect();
    let uris: Vec<String> = tracks.iter().map(|t| t.uri.clone()).collect();
    let single = tracks.len() == 1;
    let mut entries = Vec::new();

    if single {
        entries.push(MenuEntry::item("play", move |w| activate(w, row)));
    } else {
        let (tracks, view) = (playable.clone(), view.clone());
        entries.push(MenuEntry::item(format!("play these {}", tracks.len()), move |_| {
            core::spawn(async move {
                if let Err(e) = playback::play_from(tracks, 0, view.key(), view.title()).await {
                    core().error(e);
                }
            });
        }));
    }
    if playable.is_empty() {
        entries.push(MenuEntry::disabled("play next"));
    } else {
        let tracks = playable.clone();
        entries.push(MenuEntry::item("play next", move |w| play_next(w, tracks)));
    }
    if single {
        let track = tracks[0].clone();
        entries.push(MenuEntry::item("start radio", move |w| start_radio(w, Some(track))));
    }
    entries.push(MenuEntry::Separator);
    {
        let uris = uris.clone();
        entries.push(MenuEntry::item("add to playlist  \u{203a}", move |w| playlist_menu(w, row, uris)));
    }
    if let View::Source { id, name, owned: true } = &view
        && id != LIKED
    {
        let (id, name, uris) = (id.clone(), name.clone(), uris.clone());
        entries.push(MenuEntry::item("remove from this playlist", move |_| {
            remove_from_playlist(id, name, uris)
        }));
    }
    let in_liked = view.key() == LIKED;
    let label = if in_liked { "remove from liked songs" } else { "add to liked songs" };
    entries.push(MenuEntry::item(label, move |_| set_liked_many(uris, !in_liked)));
    if single {
        let track = &tracks[0];
        let has_artist = track.artists.iter().any(|a| a.id.is_some());
        let has_album = track.album.id.is_some();
        if has_artist || has_album {
            entries.push(MenuEntry::Separator);
        }
        if has_artist {
            entries.push(MenuEntry::item("go to artist", move |w| open_artist_at(w, row)));
        }
        if has_album {
            entries.push(MenuEntry::item("go to album", move |w| open_album_at(w, row)));
        }
    }
    menu::show(window, entries);
}

/// "add to playlist": the user's own playlists and a new one.
fn playlist_menu(window: &AppWindow, row: usize, uris: Vec<String>) {
    let (playlists, showing) = with_state(|s| {
        let showing = match s.view() {
            Some(View::Source { id, .. }) => Some(id.clone()),
            _ => None,
        };
        let own: Vec<(String, String)> = s
            .playlists
            .iter()
            .filter(|p| p.owned)
            .map(|p| (p.id.clone(), p.name.clone()))
            .collect();
        (own, showing)
    });
    let mut entries = vec![
        MenuEntry::item("\u{2039} back", move |w| row_menu(w, row)),
        MenuEntry::Separator,
    ];
    {
        let uris = uris.clone();
        entries.push(MenuEntry::item("+ new playlist\u{2026}", move |w| open_new_playlist(w, uris)));
    }
    for (id, name) in playlists {
        if showing.as_deref() == Some(id.as_str()) {
            continue;
        }
        let uris = uris.clone();
        let label = name.clone();
        entries.push(MenuEntry::item(label, move |_| add_to_playlist(id, name, uris)));
    }
    menu::show(window, entries);
}

fn play_next(window: &AppWindow, tracks: Vec<Track>) {
    let message = match tracks.as_slice() {
        [track] => format!("next up: {}", track.name),
        _ => format!("next up: {}", count(tracks.len())),
    };
    show_notice(window, message);
    core::spawn(async move {
        if let Err(e) = playback::play_next(tracks).await {
            core().error(e);
        }
    });
}

/// "e": plays the selected tracks (or the cursor's) next.
pub fn play_selected_next(window: &AppWindow) {
    let Ok(row) = usize::try_from(window.get_cursor()) else { return };
    let tracks: Vec<Track> = tracks_at(&rows_for(row)).into_iter().filter(|t| t.playable).collect();
    if !tracks.is_empty() {
        play_next(window, tracks);
    }
}

fn add_to_playlist(id: String, name: String, uris: Vec<String>) {
    core::spawn(async move {
        let asked = uris.len();
        let result = match core().not_in_playlist(&id, &uris).await {
            Ok(uris) if uris.is_empty() => Ok(uris),
            Ok(uris) => core().add_to_playlist(&id, &uris).await.map(|()| uris),
            Err(e) => Err(e),
        };
        ui::with_window(move |w| match result {
            Ok(uris) if uris.is_empty() => show_notice(w, already_there(asked, &name)),
            Ok(uris) => {
                with_state(|s| {
                    if let Some(p) = s.playlists.iter_mut().find(|p| p.id == id) {
                        p.track_count = p.track_count.map(|c| c + uris.len() as u64);
                    }
                });
                reload_list(w, &id);
                show_notice(w, added(uris.len(), asked, &name));
            }
            Err(e) => show_error(w, e),
        });
    });
}

fn remove_from_playlist(id: String, name: String, uris: Vec<String>) {
    core::spawn(async move {
        let result = core().remove_from_playlist(&id, &uris).await;
        ui::with_window(move |w| match result {
            Ok(()) => {
                // Drop them here too rather than reloading the whole list.
                let gone: HashSet<&String> = uris.iter().collect();
                let update = with_state(|s| {
                    let Some(ViewData::Tracks { tracks, album }) = s.cache.get(&id).cloned() else { return None };
                    let before = tracks.len();
                    let tracks: Vec<Track> = tracks.into_iter().filter(|t| !gone.contains(&t.uri)).collect();
                    let removed = (before - tracks.len()) as u64;
                    if let Some(p) = s.playlists.iter_mut().find(|p| p.id == id) {
                        p.track_count = p.track_count.map(|c| c.saturating_sub(removed));
                    }
                    let data = ViewData::Tracks { tracks, album };
                    s.cache.insert(id.clone(), data.clone());
                    s.view().is_some_and(|v| v.key() == id).then_some(data)
                });
                if let Some(data) = update {
                    let cursor = w.get_cursor();
                    render(w, data);
                    let last = with_state(|s| s.entries.len()) as i32 - 1;
                    w.set_cursor(cursor.min(last));
                }
                refresh_sources(w);
                show_notice(w, format!("removed {} from {name}", count(uris.len())));
            }
            Err(e) => show_error(w, e),
        });
    });
}

fn set_liked_many(uris: Vec<String>, liked: bool) {
    core::spawn(async move {
        let asked = uris.len();
        let uris = if liked {
            match core().not_liked(&uris).await {
                Ok(fresh) => fresh,
                Err(e) => return ui::with_window(move |w| show_error(w, e)),
            }
        } else {
            uris
        };
        if uris.is_empty() {
            return ui::with_window(move |w| show_notice(w, already_there(asked, "liked songs")));
        }
        let mut result = Ok(());
        for uri in &uris {
            result = core().set_liked(uri, liked).await;
            if result.is_err() {
                break;
            }
        }
        ui::with_window(move |w| {
            // The like button, if one of them is playing.
            let playing = with_state(|s| s.liked_uri.clone()).filter(|u| uris.contains(u));
            if let (Some(uri), Ok(())) = (playing, &result) {
                set_liked_state(w, &uri, liked);
            }
            reload_list(w, LIKED);
            match result {
                Ok(()) if liked => show_notice(w, added(uris.len(), asked, "liked songs")),
                Ok(()) => show_notice(w, format!("removed {} from liked songs", count(uris.len()))),
                Err(e) => show_error(w, e),
            }
        });
    });
}

/// "added 3 tracks to gym", noting any that were there already.
fn added(new: usize, asked: usize, place: &str) -> String {
    match asked.saturating_sub(new) {
        0 => format!("added {} to {place}", count(new)),
        skipped => format!("added {} to {place}; {} already there", count(new), skipped),
    }
}

fn already_there(asked: usize, place: &str) -> String {
    if asked == 1 { format!("already in {place}") } else { format!("all {asked} tracks are already in {place}") }
}

/// Asks for a name; `uris` go into the new playlist.
fn open_new_playlist(window: &AppWindow, uris: Vec<String>) {
    let hint = if uris.is_empty() { String::new() } else { format!(" and add {} to it", count(uris.len())) };
    window.set_new_playlist_hint(hint.into());
    with_state(|s| s.new_playlist_uris = uris);
    window.set_overlay(Overlay::NewPlaylist);
}

fn create_playlist(window: &AppWindow, name: String) {
    window.set_overlay(Overlay::None);
    window.invoke_focus_keys();
    let uris = with_state(|s| std::mem::take(&mut s.new_playlist_uris));
    core::spawn(async move {
        let result = core().create_playlist(&name).await;
        ui::with_window(move |w| match result {
            Ok(playlist) => {
                let id = playlist.id.clone();
                // Spotify lists new playlists first.
                with_state(|s| s.playlists.insert(0, playlist));
                refresh_sources(w);
                if uris.is_empty() {
                    show_notice(w, format!("created {name}"));
                } else {
                    add_to_playlist(id, name, uris);
                }
            }
            Err(e) => show_error(w, e),
        });
    });
}
