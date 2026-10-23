//! Lyrics from LRCLIB (https://lrclib.net), a free, keyless lyrics database
//! with time-synced (LRC) lyrics.

use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde::Serialize;
use serde_json::Value;

const API: &str = "https://lrclib.net/api";
const USER_AGENT: &str = concat!("spotini/", env!("CARGO_PKG_VERSION"), " (https://github.com/berkutercin/spotini)");
/// How far a match's length may be from the track's, in seconds.
const DURATION_TOLERANCE: f64 = 3.0;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Line {
    pub time_ms: u64,
    pub text: String,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Lyrics {
    /// Time-synced lines, when available.
    pub synced: Option<Vec<Line>>,
    /// Plain text, used when there are no synced lines.
    pub plain: Option<String>,
    pub instrumental: bool,
}

pub struct LyricsQuery<'a> {
    pub track: &'a str,
    pub artist: &'a str,
    pub album: &'a str,
    pub duration_ms: u64,
}

/// GETs a LRCLIB endpoint; `None` on 404. Retries once when LRCLIB says it's busy.
async fn get(client: &Client, path: &str, params: &[(&str, String)]) -> Result<Option<Value>, String> {
    for attempt in 0..2 {
        let response = client
            .get(format!("{API}{path}"))
            .query(params)
            .header("User-Agent", USER_AGENT)
            .send()
            .await
            .map_err(|e| format!("Could not reach LRCLIB: {e}"))?;
        match response.status() {
            StatusCode::NOT_FOUND => return Ok(None),
            StatusCode::SERVICE_UNAVAILABLE | StatusCode::TOO_MANY_REQUESTS if attempt == 0 => {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            status if status.is_success() => return response.json().await.map(Some).map_err(|e| e.to_string()),
            status => return Err(format!("LRCLIB error {status}")),
        }
    }
    Err("LRCLIB is busy, try again in a moment".into())
}

/// Drops decorations that rarely match across services, like "- Remastered 2011" or "(feat. X)".
fn simplify_title(title: &str) -> &str {
    let cut = [" - ", " (", " ["]
        .iter()
        .filter_map(|sep| title.find(sep))
        .min()
        .unwrap_or(title.len());
    title[..cut].trim()
}

pub async fn find(client: &Client, query: LyricsQuery<'_>) -> Result<Lyrics, String> {
    let duration_s = query.duration_ms as f64 / 1000.0;
    // LRCLIB often answers a miss with 503 rather than 404, so any failure here
    // just means "no exact match" and we fall through to searching.
    let exact = get(
        client,
        "/get",
        &[
            ("track_name", query.track.to_string()),
            ("artist_name", query.artist.to_string()),
            ("album_name", query.album.to_string()),
            ("duration", duration_s.round().to_string()),
        ],
    )
    .await;
    if let Ok(Some(record)) = exact {
        return Ok(to_lyrics(&record));
    }

    // No exact match: search loosely and take the closest-length result, preferring synced lyrics.
    let results = get(
        client,
        "/search",
        &[
            ("track_name", simplify_title(query.track).to_string()),
            ("artist_name", query.artist.to_string()),
        ],
    )
    .await?
    .unwrap_or_default();
    let best = results
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| {
            r["duration"]
                .as_f64()
                .is_some_and(|d| (d - duration_s).abs() <= DURATION_TOLERANCE)
        })
        .max_by_key(|r| (r["syncedLyrics"].is_string(), r["plainLyrics"].is_string()));
    Ok(best.map(to_lyrics).unwrap_or_default())
}

fn to_lyrics(record: &Value) -> Lyrics {
    let synced = record["syncedLyrics"]
        .as_str()
        .map(parse_lrc)
        .filter(|lines| !lines.is_empty());
    Lyrics {
        synced,
        plain: record["plainLyrics"].as_str().map(str::to_string),
        instrumental: record["instrumental"].as_bool().unwrap_or(false),
    }
}

/// Parses LRC lines like `[01:23.45] text`, including lines with several
/// timestamps. Metadata tags such as `[ar: …]` are skipped.
fn parse_lrc(lrc: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    for raw in lrc.lines() {
        let mut rest = raw.trim();
        let mut times = Vec::new();
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some(end) = stripped.find(']') else { break };
            match parse_timestamp(&stripped[..end]) {
                Some(ms) => times.push(ms),
                None => break,
            }
            rest = &stripped[end + 1..];
        }
        let text = rest.trim().to_string();
        lines.extend(times.into_iter().map(|time_ms| Line {
            time_ms,
            text: text.clone(),
        }));
    }
    lines.sort_by_key(|l| l.time_ms);
    lines
}

/// `mm:ss.xx` (or `mm:ss`) to milliseconds.
fn parse_timestamp(stamp: &str) -> Option<u64> {
    let (minutes, seconds) = stamp.split_once(':')?;
    let minutes: u64 = minutes.trim().parse().ok()?;
    let seconds: f64 = seconds.trim().parse().ok()?;
    Some(minutes * 60_000 + (seconds * 1000.0).round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lrc_lines() {
        let lines = parse_lrc("[ar: Someone]\n[00:01.50] first\n[00:10.00][01:02.25]chorus\n[00:05.00]\n");
        let got: Vec<_> = lines.iter().map(|l| (l.time_ms, l.text.as_str())).collect();
        assert_eq!(got, vec![(1500, "first"), (5000, ""), (10000, "chorus"), (62250, "chorus")]);
    }

    /// Hits the real LRCLIB; run with `cargo test -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn finds_synced_lyrics_for_a_remastered_title() {
        let query = LyricsQuery {
            track: "Bohemian Rhapsody - Remastered 2011",
            artist: "Queen",
            album: "A Night At The Opera (2011 Remaster)",
            duration_ms: 354_947,
        };
        let lyrics = find(&Client::new(), query).await.expect("lookup succeeds");
        assert!(lyrics.synced.is_some_and(|lines| lines.len() > 20));
    }

    #[test]
    fn simplifies_titles() {
        assert_eq!(simplify_title("Song - Remastered 2011"), "Song");
        assert_eq!(simplify_title("Song (feat. X)"), "Song");
        assert_eq!(simplify_title("Plain"), "Plain");
    }
}
