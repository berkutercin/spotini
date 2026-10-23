//! Spotify's internal endpoints, through librespot's session: what the Web API
//! no longer gives development-mode apps (followed playlists' tracks, song
//! radio). Unofficial, like playback itself; they can change without notice.

use librespot_core::{Session, SpotifyId, SpotifyUri};
use librespot_metadata::image::ImageSize;
use librespot_metadata::{Metadata, Playlist, Track as SpotifyTrack};
use tokio::task::JoinSet;

use crate::library::{NamedRef, Track};

/// Track metadata requests in flight at once.
const PARALLEL: usize = 16;
/// Tracks asked for per radio request (Spotify's own default).
const RADIO_COUNT: usize = 50;
/// Recently queued tracks sent along so the station doesn't repeat them.
const RADIO_HISTORY: usize = 50;

fn base62_id(uri: &SpotifyUri) -> Option<String> {
    uri.to_uri().ok()?.rsplit(':').next().map(str::to_string)
}

fn to_track(t: &SpotifyTrack) -> Option<Track> {
    let covers = &t.album.covers;
    let cover = covers
        .iter()
        .find(|c| c.size == ImageSize::SMALL)
        .or_else(|| covers.first())
        .and_then(|c| c.id.to_base16().ok())
        .map(|hex| format!("https://i.scdn.co/image/{hex}"));
    Some(Track::new(
        t.id.to_uri().ok()?,
        t.name.clone(),
        t.artists
            .iter()
            .map(|a| NamedRef::new(base62_id(&a.id), a.name.clone()))
            .collect(),
        NamedRef::new(base62_id(&t.album.id), t.album.name.clone()),
        cover,
        t.duration.max(0) as u64,
    ))
}

/// Full track details for `uris`, in the same order; tracks that fail to load are left out.
async fn tracks(session: &Session, uris: Vec<SpotifyUri>) -> Vec<Track> {
    let mut found: Vec<Option<Track>> = vec![None; uris.len()];
    for (chunk_index, chunk) in uris.chunks(PARALLEL).enumerate() {
        let mut set = JoinSet::new();
        for (i, uri) in chunk.iter().enumerate() {
            let (session, uri) = (session.clone(), uri.clone());
            set.spawn(async move { (i, SpotifyTrack::get(&session, &uri).await) });
        }
        while let Some(Ok((i, result))) = set.join_next().await {
            found[chunk_index * PARALLEL + i] = result.ok().as_ref().and_then(to_track);
        }
    }
    found.into_iter().flatten().collect()
}

/// The tracks of any playlist, including ones the user only follows.
pub async fn playlist_tracks(session: &Session, playlist_id: &str) -> Result<Vec<Track>, String> {
    let uri = SpotifyUri::from_uri(&format!("spotify:playlist:{playlist_id}")).map_err(|e| e.to_string())?;
    let playlist = Playlist::get(session, &uri).await.map_err(|e| format!("Could not load the playlist: {e}"))?;
    let uris = playlist
        .tracks()
        .filter(|u| matches!(u, SpotifyUri::Track { .. }))
        .cloned()
        .collect();
    Ok(tracks(session, uris).await)
}

/// A radio station seeded by a track: songs like it, skipping `played`.
pub async fn radio(session: &Session, seed_uri: &str, played: &[String]) -> Result<Vec<Track>, String> {
    let previous: Vec<SpotifyId> = played
        .iter()
        .rev()
        .take(RADIO_HISTORY)
        .filter_map(|u| SpotifyId::from_base62(u.rsplit(':').next()?).ok())
        .collect();
    let bytes = session
        .spclient()
        .get_apollo_station("stations", seed_uri, Some(RADIO_COUNT), previous, false)
        .await
        .map_err(|e| format!("Could not start the radio: {e}"))?;
    let station: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let uris = station["tracks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| SpotifyUri::from_uri(t["uri"].as_str()?).ok())
        .collect();
    Ok(tracks(session, uris).await)
}
