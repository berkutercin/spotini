//! The demo's stand-in for the app's core: the made-up library instead of
//! Spotify, and covers drawn instead of downloaded.

use std::path::PathBuf;
use std::sync::OnceLock;

use slint::{Rgba8Pixel, SharedPixelBuffer};

use crate::library::{Album, Artist, Playlist, SearchResults, Track};
use crate::lyrics::{Lyrics, LyricsQuery};
use crate::settings::{self, CoverColors};
use crate::{data, library_view, ui};

/// The browser's clock; the standard one doesn't work there.
pub use web_time::Instant;

const COVER_SIZE: u32 = 112;

pub struct Core {
    /// Where settings would be saved. A browser page has no files, so they aren't.
    pub config_dir: PathBuf,
}

static CORE: OnceLock<Core> = OnceLock::new();

pub fn core() -> &'static Core {
    CORE.get_or_init(|| Core {
        config_dir: PathBuf::from("/spotini"),
    })
}

pub fn spawn(future: impl Future<Output = ()> + 'static) {
    let _ = slint::spawn_local(future);
}

impl Core {
    /// Shows an error above the player once the current event is handled, as the app does.
    pub fn error(&self, message: impl Into<String>) {
        let message = message.into();
        let _ = slint::invoke_from_event_loop(move || ui::with_window(move |w| library_view::show_error(w, message)));
    }

    pub async fn playlists(&self) -> Result<Vec<Playlist>, String> {
        Ok(data::playlists())
    }

    pub async fn playlist_tracks(&self, playlist_id: &str, _owned: bool) -> Result<Vec<Track>, String> {
        data::playlist_tracks(playlist_id)
    }

    pub async fn liked_tracks(&self) -> Result<Vec<Track>, String> {
        Ok(data::liked_tracks())
    }

    pub async fn search(&self, query: &str, page: u32) -> Result<SearchResults, String> {
        Ok(data::search(query, page))
    }

    pub async fn album(&self, album_id: &str) -> Result<Album, String> {
        Ok(data::album(album_id))
    }

    pub async fn artist(&self, artist_id: &str) -> Result<Artist, String> {
        Ok(data::artist_page(artist_id))
    }

    pub async fn is_liked(&self, uri: String) -> Result<bool, String> {
        Ok(data::is_liked(&uri))
    }

    pub async fn set_liked(&self, uri: &str, liked: bool) -> Result<(), String> {
        data::set_liked(uri, liked);
        Ok(())
    }

    pub async fn not_liked(&self, uris: &[String]) -> Result<Vec<String>, String> {
        Ok(without_repeats(uris).into_iter().filter(|uri| !data::is_liked(uri)).collect())
    }

    pub async fn not_in_playlist(&self, playlist_id: &str, uris: &[String]) -> Result<Vec<String>, String> {
        let present: Vec<String> = data::playlist_tracks(playlist_id)?.into_iter().map(|t| t.uri).collect();
        Ok(without_repeats(uris).into_iter().filter(|uri| !present.contains(uri)).collect())
    }

    pub async fn create_playlist(&self, name: &str) -> Result<Playlist, String> {
        Ok(data::create_playlist(name))
    }

    pub async fn add_to_playlist(&self, playlist_id: &str, uris: &[String]) -> Result<(), String> {
        data::add_to_playlist(playlist_id, uris)
    }

    pub async fn remove_from_playlist(&self, playlist_id: &str, uris: &[String]) -> Result<(), String> {
        data::remove_from_playlist(playlist_id, uris)
    }

    pub async fn lyrics(&self, query: LyricsQuery<'_>) -> Result<Lyrics, String> {
        Ok(data::lyrics(query.track, query.duration_ms))
    }

    /// Draws a made-up cover: a two-colour gradient with a soft circle.
    pub async fn cover(&self, url: &str) -> Result<(SharedPixelBuffer<Rgba8Pixel>, Option<CoverColors>), String> {
        let seed: usize = url.strip_prefix("demo:cover:").and_then(|s| s.parse().ok()).ok_or("No such cover")?;
        let hue = (seed * 47 % 360) as f32;
        let from = hsl(hue, 0.7, 0.58);
        let to = hsl((hue + 70.0) % 360.0, 0.6, 0.28);
        let scale = COVER_SIZE as f32 / 64.0;
        let (cx, cy) = ((20 + seed % 3 * 12) as f32 * scale, (22 + seed % 4 * 7) as f32 * scale);
        let radius = (10 + seed % 5 * 3) as f32 * scale;
        let last = (COVER_SIZE - 1) as f32;
        let mut pixels = SharedPixelBuffer::<Rgba8Pixel>::new(COVER_SIZE, COVER_SIZE);
        for (i, pixel) in pixels.make_mut_slice().iter_mut().enumerate() {
            let (x, y) = ((i as u32 % COVER_SIZE) as f32, (i as u32 / COVER_SIZE) as f32);
            let t = (x + y) / (2.0 * last);
            let mut rgb: [f32; 3] = std::array::from_fn(|c| from[c] + (to[c] - from[c]) * t);
            if (x - cx).powi(2) + (y - cy).powi(2) <= radius * radius {
                rgb = rgb.map(|v| v + (255.0 - v) * 0.18);
            }
            *pixel = Rgba8Pixel::new(rgb[0] as u8, rgb[1] as u8, rgb[2] as u8, 255);
        }
        let rgba: Vec<u8> = pixels.as_slice().iter().flat_map(|p| [p.r, p.g, p.b, p.a]).collect();
        let colors = settings::cover_colors(&rgba);
        Ok((pixels, colors))
    }
}

/// HSL (hue in degrees) to RGB, 0–255.
fn hsl(hue: f32, saturation: f32, lightness: f32) -> [f32; 3] {
    let c = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let h = hue / 60.0;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = lightness - c / 2.0;
    [(r + m) * 255.0, (g + m) * 255.0, (b + m) * 255.0]
}

fn without_repeats(uris: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    uris.iter().filter(|uri| seen.insert(uri.as_str())).cloned().collect()
}
