//! The user's library, read from the Web API and flattened into the shapes the
//! frontend uses.

use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::web_api::{WebApi, api_url};

/// Spotify caps search pages at 10 results for development-mode apps.
const SEARCH_PAGE: u32 = 10;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub image_url: Option<String>,
    pub track_count: Option<u64>,
    /// Owned or collaborative. Other playlists' tracks come from Spotify's internal API.
    pub owned: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct NamedRef {
    pub id: Option<String>,
    pub name: String,
}

impl NamedRef {
    pub fn new(id: Option<String>, name: String) -> Self {
        Self { id, name }
    }
}

/// A track as listed in the UI. The frontend hands these back to start a queue.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub uri: String,
    pub name: String,
    pub artists: Vec<NamedRef>,
    pub album: NamedRef,
    pub image_url: Option<String>,
    pub duration_ms: u64,
    pub playable: bool,
}

impl Track {
    pub fn new(
        uri: String,
        name: String,
        artists: Vec<NamedRef>,
        album: NamedRef,
        image_url: Option<String>,
        duration_ms: u64,
    ) -> Self {
        Self {
            uri,
            name,
            artists,
            album,
            image_url,
            duration_ms,
            playable: true,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
impl Track {
    pub fn for_test(uri: &str, playable: bool) -> Self {
        Self {
            uri: uri.to_string(),
            name: uri.to_string(),
            artists: Vec::new(),
            album: NamedRef { id: None, name: String::new() },
            image_url: None,
            duration_ms: 1000,
            playable,
        }
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AlbumSummary {
    pub id: String,
    pub name: String,
    pub artists: Vec<NamedRef>,
    pub image_url: Option<String>,
    pub album_type: String,
    pub year: Option<String>,
    pub total_tracks: Option<u64>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ArtistSummary {
    pub id: String,
    pub name: String,
    pub image_url: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SearchResults {
    pub tracks: Vec<Track>,
    pub albums: Vec<AlbumSummary>,
    pub artists: Vec<ArtistSummary>,
    pub has_more: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Album {
    #[serde(flatten)]
    pub summary: AlbumSummary,
    pub tracks: Vec<Track>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Artist {
    #[serde(flatten)]
    pub summary: ArtistSummary,
    pub albums: Vec<AlbumSummary>,
}

/// Picks the smallest image at least `min_width` wide; images without a
/// width (playlist mosaics) count as large.
fn pick_image(images: &Value, min_width: u64) -> Option<String> {
    let images = images.as_array()?;
    images
        .iter()
        .filter(|img| img["width"].as_u64().is_none_or(|w| w >= min_width))
        .min_by_key(|img| img["width"].as_u64().unwrap_or(u64::MAX))
        .or_else(|| images.first())
        .and_then(|img| img["url"].as_str())
        .map(str::to_string)
}

fn named_ref(value: &Value) -> NamedRef {
    NamedRef {
        id: value["id"].as_str().map(str::to_string),
        name: value["name"].as_str().unwrap_or_default().to_string(),
    }
}

fn to_track(track: &Value) -> Option<Track> {
    if track["type"].as_str() != Some("track") {
        return None;
    }
    let is_local = track["is_local"].as_bool().unwrap_or(false);
    Some(Track {
        uri: track["uri"].as_str()?.to_string(),
        name: track["name"].as_str().unwrap_or_default().to_string(),
        artists: track["artists"]
            .as_array()
            .map(|artists| artists.iter().map(named_ref).collect())
            .unwrap_or_default(),
        album: named_ref(&track["album"]),
        image_url: pick_image(&track["album"]["images"], 64),
        duration_ms: track["duration_ms"].as_u64().unwrap_or_default(),
        playable: !is_local && track["is_playable"].as_bool().unwrap_or(true),
    })
}

fn to_album_summary(album: &Value) -> Option<AlbumSummary> {
    Some(AlbumSummary {
        id: album["id"].as_str()?.to_string(),
        name: album["name"].as_str().unwrap_or_default().to_string(),
        artists: album["artists"]
            .as_array()
            .map(|artists| artists.iter().map(named_ref).collect())
            .unwrap_or_default(),
        image_url: pick_image(&album["images"], 64),
        album_type: album["album_type"].as_str().unwrap_or("album").to_string(),
        year: album["release_date"].as_str().map(|d| d.chars().take(4).collect()),
        total_tracks: album["total_tracks"].as_u64(),
    })
}

fn to_artist_summary(artist: &Value) -> Option<ArtistSummary> {
    Some(ArtistSummary {
        id: artist["id"].as_str()?.to_string(),
        name: artist["name"].as_str().unwrap_or_default().to_string(),
        image_url: pick_image(&artist["images"], 64),
    })
}

fn items(paging: &Value) -> impl Iterator<Item = &Value> {
    paging["items"].as_array().into_iter().flatten()
}

/// One page of search results; `page` counts from 0.
pub async fn search(api: &WebApi, query: &str, page: u32) -> Result<SearchResults, String> {
    let limit = SEARCH_PAGE.to_string();
    let offset = (page * SEARCH_PAGE).to_string();
    let url = api_url(
        "/search",
        &[("q", query), ("type", "track,album,artist"), ("limit", &limit), ("offset", &offset)],
    );
    let results = api.get(&url).await?;

    let has_more = ["tracks", "albums", "artists"]
        .iter()
        .any(|kind| results[kind]["next"].is_string());
    Ok(SearchResults {
        tracks: items(&results["tracks"]).filter_map(to_track).collect(),
        albums: items(&results["albums"]).filter_map(to_album_summary).collect(),
        artists: items(&results["artists"]).filter_map(to_artist_summary).collect(),
        has_more,
    })
}

pub async fn album(api: &WebApi, album_id: &str) -> Result<Album, String> {
    let mut album = api.get(&format!("/albums/{album_id}")).await?;
    let mut track_values: Vec<Value> = items(&album["tracks"]).cloned().collect();
    if let Some(next) = album["tracks"]["next"].as_str() {
        track_values.extend(api.get_all(next).await?);
    }

    // Album tracks come without their album; attach it so they look like any other track.
    album["tracks"] = Value::Null;
    let tracks = track_values
        .into_iter()
        .filter_map(|mut track| {
            track["album"] = album.clone();
            to_track(&track)
        })
        .collect();

    Ok(Album {
        summary: to_album_summary(&album).ok_or("Spotify returned an album without an ID")?,
        tracks,
    })
}

/// The artist with their albums and singles. Spotify no longer offers top
/// tracks to development-mode apps.
pub async fn artist(api: &WebApi, artist_id: &str) -> Result<Artist, String> {
    let artist = api.get(&format!("/artists/{artist_id}")).await?;
    let albums = api
        .get_all(&api_url(
            &format!("/artists/{artist_id}/albums"),
            &[("include_groups", "album,single"), ("limit", "10")],
        ))
        .await?;

    Ok(Artist {
        summary: to_artist_summary(&artist).ok_or("Spotify returned an artist without an ID")?,
        albums: albums.iter().filter_map(to_album_summary).collect(),
    })
}

fn track_id(uri: &str) -> &str {
    uri.rsplit(':').next().unwrap_or(uri)
}

/// Whether each track is in the user's Liked Songs. Uses the February 2026
/// library endpoint and falls back to the older tracks endpoint.
pub async fn liked(api: &WebApi, uris: &[String]) -> Result<Vec<bool>, String> {
    let as_bools = |value: Value| -> Option<Vec<bool>> {
        value.as_array()?.iter().map(Value::as_bool).collect()
    };
    let joined = uris.join(",");
    if let Some(flags) = api
        .get(&api_url("/me/library/contains", &[("uris", &joined)]))
        .await
        .ok()
        .and_then(as_bools)
    {
        return Ok(flags);
    }
    let ids: Vec<&str> = uris.iter().map(|uri| track_id(uri)).collect();
    let value = api
        .get(&api_url("/me/tracks/contains", &[("ids", &ids.join(","))]))
        .await?;
    as_bools(value).ok_or_else(|| "Unexpected answer from Spotify".to_string())
}

/// Adds a track to, or removes it from, Liked Songs.
pub async fn set_liked(api: &WebApi, uri: &str, liked: bool) -> Result<(), String> {
    let method = if liked { Method::PUT } else { Method::DELETE };
    let current = api
        .request(method.clone(), &api_url("/me/library", &[("uris", uri)]))
        .await;
    if current.is_ok() {
        return Ok(());
    }
    api.request(method, &api_url("/me/tracks", &[("ids", track_id(uri))]))
        .await
        .map(|_| ())
        .map_err(|legacy| current.err().unwrap_or(legacy))
}

/// The user's playlists, owned and followed. Spotify only returns the contents
/// of owned ones to development-mode apps; see `internal` for the rest.
pub async fn playlists(api: &WebApi) -> Result<Vec<Playlist>, String> {
    let me = api.get("/me").await?;
    let my_id = me["id"].as_str().unwrap_or_default();

    let playlists = api.get_all("/me/playlists?limit=50").await?;
    Ok(playlists
        .iter()
        .filter_map(|p| {
            Some(Playlist {
                id: p["id"].as_str()?.to_string(),
                name: p["name"].as_str().unwrap_or_default().to_string(),
                image_url: pick_image(&p["images"], 64),
                // Renamed from `tracks` to `items` in the February 2026 API changes.
                track_count: p["items"]["total"].as_u64().or_else(|| p["tracks"]["total"].as_u64()),
                owned: p["owner"]["id"].as_str() == Some(my_id) || p["collaborative"].as_bool() == Some(true),
            })
        })
        .collect())
}

pub async fn playlist_tracks(api: &WebApi, playlist_id: &str) -> Result<Vec<Track>, String> {
    let items = api
        .get_all(&format!("/playlists/{playlist_id}/items?limit=50&additional_types=track"))
        .await?;
    Ok(items
        .iter()
        .filter_map(|entry| {
            let track = if entry["item"].is_object() { &entry["item"] } else { &entry["track"] };
            to_track(track)
        })
        .collect())
}

pub async fn liked_tracks(api: &WebApi) -> Result<Vec<Track>, String> {
    let items = api.get_all("/me/tracks?limit=50").await?;
    Ok(items.iter().filter_map(|entry| to_track(&entry["track"])).collect())
}

/// Spotify takes at most this many items per playlist change.
const PLAYLIST_BATCH: usize = 100;

/// Makes a new private playlist. Tries the February 2026 endpoint first.
pub async fn create_playlist(api: &WebApi, name: &str) -> Result<Playlist, String> {
    let body = serde_json::json!({ "name": name, "public": false });
    let created = match api.request_json(Method::POST, "/me/playlists", &body).await {
        Ok(created) => created,
        Err(current) => {
            let me = api.get("/me").await?;
            let my_id = me["id"].as_str().ok_or(current.clone())?;
            api.request_json(Method::POST, &format!("/users/{my_id}/playlists"), &body)
                .await
                .map_err(|_| current)?
        }
    };
    Ok(Playlist {
        id: created["id"].as_str().ok_or("Unexpected answer from Spotify")?.to_string(),
        name: created["name"].as_str().unwrap_or(name).to_string(),
        image_url: None,
        track_count: Some(0),
        owned: true,
    })
}

/// Sends a playlist change in batches, to the `items` endpoint or else the older `tracks` one.
async fn change_playlist(
    api: &WebApi,
    method: Method,
    playlist_id: &str,
    uris: &[String],
    body: impl Fn(&[String], bool) -> Value,
) -> Result<(), String> {
    for batch in uris.chunks(PLAYLIST_BATCH) {
        let current = api
            .request_json(method.clone(), &format!("/playlists/{playlist_id}/items"), &body(batch, true))
            .await;
        if current.is_ok() {
            continue;
        }
        api.request_json(method.clone(), &format!("/playlists/{playlist_id}/tracks"), &body(batch, false))
            .await
            .map_err(|legacy| current.err().unwrap_or(legacy))?;
    }
    Ok(())
}

pub async fn add_to_playlist(api: &WebApi, playlist_id: &str, uris: &[String]) -> Result<(), String> {
    change_playlist(api, Method::POST, playlist_id, uris, |batch, _| serde_json::json!({ "uris": batch })).await
}

/// Removes every occurrence of the tracks from a playlist.
pub async fn remove_from_playlist(api: &WebApi, playlist_id: &str, uris: &[String]) -> Result<(), String> {
    change_playlist(api, Method::DELETE, playlist_id, uris, |batch, current| {
        let items: Vec<Value> = batch.iter().map(|uri| serde_json::json!({ "uri": uri })).collect();
        // The list was renamed from `tracks` to `items` with the endpoint.
        if current {
            serde_json::json!({ "items": items })
        } else {
            serde_json::json!({ "tracks": items })
        }
    })
    .await
}
