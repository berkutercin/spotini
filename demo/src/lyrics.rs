//! Lyrics' shapes, as the app's lyrics module has them.

#[derive(Clone)]
pub struct Line {
    pub time_ms: u64,
    pub text: String,
}

#[derive(Default)]
pub struct Lyrics {
    pub synced: Option<Vec<Line>>,
    pub plain: Option<String>,
    pub instrumental: bool,
}

pub struct LyricsQuery<'a> {
    pub track: &'a str,
    pub artist: &'a str,
    pub album: &'a str,
    pub duration_ms: u64,
}
