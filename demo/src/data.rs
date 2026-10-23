//! The made-up library: artists, playlists and lyrics, the same every visit.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::library::{Album, AlbumSummary, Artist, ArtistSummary, NamedRef, Playlist, SearchResults, Track};
use crate::lyrics::{Line, Lyrics};

const ARTISTS: [&str; 7] = ["Velvet Static", "Mira Loom", "The Quiet Arcade", "Kôr", "Juno Park", "Arden Vale", "Gece Yolu"];
const TITLES: [&str; 16] = [
    "Neon Harbor", "Paper Satellites", "Cold Coffee, Warm Rain", "Midnight Telegraph", "Glass Elevator",
    "Slow Motion City", "Halcyon", "Telefon Kulübesi", "Afterimage", "Static Hearts", "Orbit Lullaby",
    "Last Train Home", "Lantern Season", "Blue Hour Radio", "Sakin Liman", "Overpass",
];
const ALBUMS: [&str; 8] = ["Wires", "Low Orbit", "Sundays", "Afterglow Tapes", "Static Bloom", "Sinyal", "Echoes", "Up"];
/// Name, track count, and whether the visitor owns it (followed ones come without a count).
const PLAYLISTS: [(&str, usize, bool); 8] = [
    ("late night drive", 24, true),
    ("focus · deep work", 58, true),
    ("türkçe rock", 37, true),
    ("gym", 34, true),
    ("rainy sunday", 21, true),
    ("discover later", 140, true),
    ("Daily Mix 1", 50, false),
    ("Lo-fi Beats", 80, false),
];
const LYRICS: [&str; 14] = [
    "the wires hum below the bridge", "a signal only we can hear", "", "you said the city never sleeps",
    "it just forgets to dream", "", "send it down the midnight telegraph", "dot dash, the lights go out",
    "send it down the midnight telegraph", "and wait for something back", "", "the rain writes letters on the glass",
    "in a hand I almost know", "",
];

/// A cover's address; `core().cover` draws it.
pub fn cover_url(seed: usize) -> String {
    format!("demo:cover:{seed}")
}

fn artist(index: usize) -> NamedRef {
    let index = index % ARTISTS.len();
    NamedRef {
        id: Some(format!("ar{index}")),
        name: ARTISTS[index].into(),
    }
}

/// Album `n`: its ID, name, artist and cover.
fn album_of(n: usize) -> (String, &'static str, NamedRef, String) {
    let i = n % ALBUMS.len();
    (format!("al{i}"), ALBUMS[i], artist(i), cover_url(i + 3))
}

fn make_tracks(count: usize, seed: usize) -> Vec<Track> {
    (0..count)
        .map(|i| {
            let (album_id, album_name, _, image) = album_of(i + seed);
            Track {
                uri: format!("spotify:track:{seed:04}{i:018}"),
                name: TITLES[(i * 7 + seed) % TITLES.len()].into(),
                artists: vec![artist(i + seed)],
                album: NamedRef {
                    id: Some(album_id),
                    name: album_name.into(),
                },
                image_url: Some(image),
                duration_ms: 150_000 + ((i * 37 + seed * 11) % 140) as u64 * 1000,
                playable: (i + seed) % 13 != 5,
            }
        })
        .collect()
}

thread_local! {
    /// Liking and unliking changes it.
    static LIKED: RefCell<Vec<Track>> = RefCell::new(make_tracks(60, 3));
    /// Playlists made this visit (ID and name), newest first.
    static CREATED: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
    /// Playlists whose tracks were changed this visit, by ID.
    static EDITED: RefCell<HashMap<String, Vec<Track>>> = RefCell::new(HashMap::new());
}

pub fn playlists() -> Vec<Playlist> {
    let count_of = |id: &str| EDITED.with_borrow(|e| e.get(id).map(|t| t.len() as u64));
    let created = CREATED.with_borrow(|c| c.clone()).into_iter().map(|(id, name)| Playlist {
        track_count: count_of(&id),
        id,
        name,
        image_url: None,
        owned: true,
    });
    let made_up = PLAYLISTS.iter().enumerate().map(|(i, &(name, count, owned))| {
        let id = format!("pl{i}");
        Playlist {
            track_count: count_of(&id).or(owned.then_some(count as u64)),
            id,
            name: name.into(),
            image_url: Some(cover_url(i + 20)),
            owned,
        }
    });
    created.chain(made_up).collect()
}

pub fn playlist_tracks(id: &str) -> Result<Vec<Track>, String> {
    if let Some(tracks) = EDITED.with_borrow(|e| e.get(id).cloned()) {
        return Ok(tracks);
    }
    let index: usize = id.strip_prefix("pl").and_then(|i| i.parse().ok()).ok_or("No such playlist")?;
    let (_, count, _) = PLAYLISTS.get(index).ok_or("No such playlist")?;
    Ok(make_tracks(*count, index + 5))
}

pub fn create_playlist(name: &str) -> Playlist {
    let id = format!("new{}", CREATED.with_borrow(|c| c.len()));
    CREATED.with_borrow_mut(|c| c.insert(0, (id.clone(), name.into())));
    EDITED.with_borrow_mut(|e| e.insert(id.clone(), Vec::new()));
    Playlist {
        id,
        name: name.into(),
        image_url: None,
        track_count: Some(0),
        owned: true,
    }
}

pub fn add_to_playlist(id: &str, uris: &[String]) -> Result<(), String> {
    let mut tracks = playlist_tracks(id)?;
    tracks.extend(uris.iter().filter_map(|uri| find_track(uri)));
    EDITED.with_borrow_mut(|e| e.insert(id.into(), tracks));
    Ok(())
}

pub fn remove_from_playlist(id: &str, uris: &[String]) -> Result<(), String> {
    let mut tracks = playlist_tracks(id)?;
    tracks.retain(|t| !uris.contains(&t.uri));
    EDITED.with_borrow_mut(|e| e.insert(id.into(), tracks));
    Ok(())
}

/// Any made-up track, from its URI: "spotify:track:" + a 4-digit seed + its index.
fn find_track(uri: &str) -> Option<Track> {
    let digits = uri.strip_prefix("spotify:track:")?;
    let seed: usize = digits.get(..4)?.parse().ok()?;
    let index: usize = digits.get(4..)?.parse().ok()?;
    // Album pages give their tracks the album's name and cover.
    if (30..30 + ALBUMS.len()).contains(&seed) {
        return album(&format!("al{}", seed - 30)).tracks.into_iter().nth(index);
    }
    make_tracks(index + 1, seed).pop()
}

pub fn liked_tracks() -> Vec<Track> {
    LIKED.with_borrow(|liked| liked.clone())
}

pub fn is_liked(uri: &str) -> bool {
    LIKED.with_borrow(|liked| liked.iter().any(|t| t.uri == uri))
}

pub fn set_liked(uri: &str, liked: bool) {
    let track = find_track(uri);
    LIKED.with_borrow_mut(|list| {
        list.retain(|t| t.uri != uri);
        if liked && let Some(track) = track {
            list.insert(0, track);
        }
    });
}

/// Every track in the library, for radios.
pub fn all_tracks() -> Vec<Track> {
    let mut tracks = liked_tracks();
    for (i, (_, count, _)) in PLAYLISTS.iter().enumerate() {
        tracks.extend(make_tracks(*count, i + 5));
    }
    tracks
}

pub fn search(query: &str, page: u32) -> SearchResults {
    let page = page as usize;
    let albums = (0..4)
        .map(|i| {
            let (id, name, artist, image) = album_of(i + page);
            AlbumSummary {
                id,
                name: name.into(),
                artists: vec![artist],
                image_url: Some(image),
                album_type: if i % 2 == 1 { "single" } else { "album" }.into(),
                year: Some((2016 + i + page).to_string()),
                total_tracks: Some(6 + i as u64 * 3),
            }
        })
        .collect();
    let artists = (0..3)
        .map(|i| {
            let artist = artist(i + page);
            ArtistSummary {
                id: artist.id.unwrap_or_default(),
                name: artist.name,
                image_url: None,
            }
        })
        .collect();
    SearchResults {
        tracks: make_tracks(10, page * 3 + query.chars().count()),
        albums,
        artists,
        has_more: page < 2,
    }
}

fn index_of(id: &str, prefix: &str) -> usize {
    id.strip_prefix(prefix).and_then(|i| i.parse().ok()).unwrap_or(0)
}

pub fn album(id: &str) -> Album {
    let i = index_of(id, "al");
    let (id, name, artist, image) = album_of(i);
    let tracks: Vec<Track> = make_tracks(9, i + 30)
        .into_iter()
        .map(|t| Track {
            album: NamedRef {
                id: Some(id.clone()),
                name: name.into(),
            },
            image_url: Some(image.clone()),
            ..t
        })
        .collect();
    Album {
        summary: AlbumSummary {
            id: id.clone(),
            name: name.into(),
            artists: vec![artist],
            image_url: Some(image.clone()),
            album_type: "album".into(),
            year: Some("2021".into()),
            total_tracks: Some(tracks.len() as u64),
        },
        tracks,
    }
}

pub fn artist_page(id: &str) -> Artist {
    let i = index_of(id, "ar");
    let me = artist(i);
    let albums = (0..6)
        .map(|k| {
            let (album_id, name, _, image) = album_of(k + i);
            AlbumSummary {
                id: album_id,
                name: name.into(),
                artists: vec![me.clone()],
                image_url: Some(image),
                album_type: if k < 3 { "album" } else { "single" }.into(),
                year: Some((2024 - k).to_string()),
                total_tracks: Some(if k < 3 { 11 } else { 2 }),
            }
        })
        .collect();
    Artist {
        summary: ArtistSummary {
            id: me.id.clone().unwrap_or_default(),
            name: me.name.clone(),
            image_url: None,
        },
        albums,
    }
}

/// Made-up lines spread over the song; "Halcyon" is an instrumental.
pub fn lyrics(track: &str, duration_ms: u64) -> Lyrics {
    if track.starts_with("Halcyon") {
        return Lyrics {
            instrumental: true,
            ..Lyrics::default()
        };
    }
    let count = duration_ms.saturating_sub(8000) / 3600;
    let synced = (0..count as usize)
        .map(|i| Line {
            time_ms: 6000 + i as u64 * 3600,
            text: LYRICS[i % LYRICS.len()].into(),
        })
        .collect();
    Lyrics {
        synced: Some(synced),
        plain: Some(LYRICS.join("\n")),
        instrumental: false,
    }
}
