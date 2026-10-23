//! The library's shapes, as the app's library module has them.

#[derive(Clone)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub image_url: Option<String>,
    pub track_count: Option<u64>,
    pub owned: bool,
}

#[derive(Clone, Debug)]
pub struct NamedRef {
    pub id: Option<String>,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct Track {
    pub uri: String,
    pub name: String,
    pub artists: Vec<NamedRef>,
    pub album: NamedRef,
    pub image_url: Option<String>,
    pub duration_ms: u64,
    pub playable: bool,
}

#[derive(Clone)]
pub struct AlbumSummary {
    pub id: String,
    pub name: String,
    pub artists: Vec<NamedRef>,
    pub image_url: Option<String>,
    pub album_type: String,
    pub year: Option<String>,
    pub total_tracks: Option<u64>,
}

#[derive(Clone)]
pub struct ArtistSummary {
    pub id: String,
    pub name: String,
    pub image_url: Option<String>,
}

#[derive(Clone)]
pub struct SearchResults {
    pub tracks: Vec<Track>,
    pub albums: Vec<AlbumSummary>,
    pub artists: Vec<ArtistSummary>,
    pub has_more: bool,
}

#[derive(Clone)]
pub struct Album {
    pub summary: AlbumSummary,
    pub tracks: Vec<Track>,
}

#[derive(Clone)]
pub struct Artist {
    pub summary: ArtistSummary,
    pub albums: Vec<AlbumSummary>,
}
