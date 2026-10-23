//! The app's shared state and the actions the UI, tray and media keys call:
//! logins, the library, and the playback engine. Lives for the whole run,
//! window or not, so music keeps playing from the tray.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use librespot_core::authentication::Credentials;
use slint::{Rgba8Pixel, SharedPixelBuffer};
use tokio::runtime::Handle;

use crate::engine::{Engine, TrackInfo};
use crate::playback::{self, PlaybackState, QueueInfo};
use crate::web_api::{self, WebApi, WebLogin};
use crate::settings::{self, CoverColors};
use crate::{auth, internal, library, lyrics};

/// The clock the screens use. The web demo, which runs the same screens, has
/// its own: the standard one doesn't work in a browser.
pub use std::time::Instant;

/// Covers are drawn at 56 px; twice that stays sharp on high-DPI screens.
const COVER_SIZE: u32 = 112;

/// What the core tells the UI.
pub enum UiEvent {
    Track(Option<TrackInfo>),
    Status { state: &'static str, position_ms: u32 },
    Queue(QueueInfo),
    Error(String),
}

pub struct Core {
    pub runtime: Handle,
    engine: tokio::sync::Mutex<Option<Engine>>,
    web: tokio::sync::Mutex<Option<Arc<WebApi>>>,
    pub http: reqwest::Client,
    /// Queue and player state. A plain mutex: it's never held across an await.
    pub playback: Mutex<PlaybackState>,
    /// Where preferences are saved.
    pub config_dir: PathBuf,
    ui: Mutex<Option<UiSink>>,
}

/// Where events for the window go.
type UiSink = Box<dyn Fn(UiEvent) + Send>;

static CORE: OnceLock<Core> = OnceLock::new();

pub fn init(runtime: Handle) {
    let config_dir = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join("dev.spotini.app");
    let _ = CORE.set(Core {
        runtime,
        engine: Default::default(),
        web: Default::default(),
        http: reqwest::Client::new(),
        playback: Default::default(),
        config_dir,
        ui: Mutex::new(None),
    });
}

pub fn core() -> &'static Core {
    CORE.get().expect("core::init runs first")
}

/// Runs `future` on the runtime, in the background.
pub fn spawn(future: impl Future<Output = ()> + Send + 'static) {
    core().runtime.spawn(future);
}

impl Core {
    /// Sets where events go; the UI forwards them to its own thread.
    pub fn set_ui(&self, ui: impl Fn(UiEvent) + Send + 'static) {
        *self.ui.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(ui));
    }

    pub fn notify(&self, event: UiEvent) {
        if let Some(ui) = self.ui.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            ui(event);
        }
    }

    pub fn error(&self, message: impl Into<String>) {
        self.notify(UiEvent::Error(message.into()));
    }

    async fn web(&self) -> Result<Arc<WebApi>, String> {
        let mut web = self.web.lock().await;
        if web.is_none() {
            let login = web_api::load().filter(WebLogin::has_token).ok_or("Library not connected")?;
            *web = Some(Arc::new(WebApi::new(login)));
        }
        Ok(web.as_ref().unwrap().clone())
    }

    async fn start_engine(&self, credentials: Credentials) -> Result<String, String> {
        let device_id = auth::device_id();
        let volume = playback::volume();
        let (engine, reusable) = Engine::connect(device_id.clone(), credentials, volume).await?;
        auth::save(&auth::StoredLogin {
            device_id,
            credentials: reusable,
        })?;
        let username = engine.username();
        if let Some(old) = self.engine.lock().await.replace(engine) {
            old.shutdown();
        }
        Ok(username)
    }

    /// Runs `f` against a live engine, reconnecting with stored credentials if
    /// Spotify dropped the session.
    pub async fn with_engine<T>(&self, f: impl FnOnce(&Engine) -> Result<T, String>) -> Result<T, String> {
        let needs_reconnect = match self.engine.lock().await.as_ref() {
            None => return Err("Not logged in".into()),
            Some(engine) => !engine.is_alive(),
        };
        if needs_reconnect {
            let stored = auth::load().ok_or("Not logged in")?;
            self.start_engine(stored.credentials).await?;
        }
        let guard = self.engine.lock().await;
        f(guard.as_ref().ok_or("Not logged in")?)
    }

    /// The playback session, for Spotify's internal endpoints.
    pub async fn session(&self) -> Result<librespot_core::Session, String> {
        self.with_engine(|engine| Ok(engine.session())).await
    }

    /// Logs in with stored credentials, if any. Returns the username.
    pub async fn restore_session(&self) -> Result<Option<String>, String> {
        if let Some(engine) = self.engine.lock().await.as_ref().filter(|e| e.is_alive()) {
            return Ok(Some(engine.username()));
        }
        let Some(stored) = auth::load() else {
            return Ok(None);
        };
        self.start_engine(stored.credentials).await.map(Some)
    }

    /// Logs in through the browser. Returns the username.
    pub async fn login(&self) -> Result<String, String> {
        let credentials = browser_flow(auth::abort_login, auth::browser_login).await?;
        self.start_engine(credentials).await
    }

    /// Logs out of playback and the library, keeping the Client ID.
    pub async fn logout(&self) -> Result<(), String> {
        if let Some(engine) = self.engine.lock().await.take() {
            engine.shutdown();
        }
        playback::reset();
        auth::clear();
        self.web.lock().await.take();
        match web_api::load() {
            Some(login) => web_api::save(&WebLogin::new(login.client_id)),
            None => {
                web_api::clear();
                Ok(())
            }
        }
    }

    pub async fn set_client_id(&self, client_id: &str) -> Result<(), String> {
        let client_id = client_id.trim().to_string();
        if !web_api::is_valid_client_id(&client_id) {
            return Err("That doesn't look like a Client ID (32 letters and digits).".into());
        }
        self.web.lock().await.take();
        web_api::save(&WebLogin::new(client_id))
    }

    /// Connects the library through the browser with the stored Client ID.
    pub async fn connect_library(&self) -> Result<(), String> {
        let client_id = web_api::load().ok_or("Set your Client ID first")?.client_id;
        let login = browser_flow(web_api::abort_login, move || web_api::browser_login(client_id)).await?;
        web_api::save(&login)?;
        *self.web.lock().await = Some(Arc::new(WebApi::new(login)));
        Ok(())
    }

    pub async fn playlists(&self) -> Result<Vec<library::Playlist>, String> {
        library::playlists(&*self.web().await?).await
    }

    pub async fn playlist_tracks(&self, playlist_id: &str, owned: bool) -> Result<Vec<library::Track>, String> {
        if owned {
            library::playlist_tracks(&*self.web().await?, playlist_id).await
        } else {
            // The Web API won't list other people's playlists for development-mode apps.
            internal::playlist_tracks(&self.session().await?, playlist_id).await
        }
    }

    pub async fn liked_tracks(&self) -> Result<Vec<library::Track>, String> {
        library::liked_tracks(&*self.web().await?).await
    }

    pub async fn search(&self, query: &str, page: u32) -> Result<library::SearchResults, String> {
        library::search(&*self.web().await?, query, page).await
    }

    pub async fn album(&self, album_id: &str) -> Result<library::Album, String> {
        library::album(&*self.web().await?, album_id).await
    }

    pub async fn artist(&self, artist_id: &str) -> Result<library::Artist, String> {
        library::artist(&*self.web().await?, artist_id).await
    }

    pub async fn is_liked(&self, uri: String) -> Result<bool, String> {
        let flags = library::liked(&*self.web().await?, &[uri]).await?;
        Ok(flags.first().copied().unwrap_or(false))
    }

    pub async fn set_liked(&self, uri: &str, liked: bool) -> Result<(), String> {
        library::set_liked(&*self.web().await?, uri, liked).await
    }

    /// The tracks in `uris` that aren't in Liked Songs yet, without repeats.
    pub async fn not_liked(&self, uris: &[String]) -> Result<Vec<String>, String> {
        let web = self.web().await?;
        let uris = without_repeats(uris);
        let mut fresh = Vec::new();
        // Spotify checks at most 40 at a time.
        for batch in uris.chunks(40) {
            let flags = library::liked(&web, batch).await?;
            fresh.extend(batch.iter().zip(flags).filter(|(_, liked)| !liked).map(|(uri, _)| uri.clone()));
        }
        Ok(fresh)
    }

    /// The tracks in `uris` that aren't in the playlist yet, without repeats.
    pub async fn not_in_playlist(&self, playlist_id: &str, uris: &[String]) -> Result<Vec<String>, String> {
        let present: std::collections::HashSet<String> = library::playlist_tracks(&*self.web().await?, playlist_id)
            .await?
            .into_iter()
            .map(|t| t.uri)
            .collect();
        Ok(without_repeats(uris).into_iter().filter(|uri| !present.contains(uri)).collect())
    }

    pub async fn create_playlist(&self, name: &str) -> Result<library::Playlist, String> {
        library::create_playlist(&*self.web().await?, name).await
    }

    pub async fn add_to_playlist(&self, playlist_id: &str, uris: &[String]) -> Result<(), String> {
        library::add_to_playlist(&*self.web().await?, playlist_id, uris).await
    }

    pub async fn remove_from_playlist(&self, playlist_id: &str, uris: &[String]) -> Result<(), String> {
        library::remove_from_playlist(&*self.web().await?, playlist_id, uris).await
    }

    pub async fn lyrics(&self, query: lyrics::LyricsQuery<'_>) -> Result<lyrics::Lyrics, String> {
        lyrics::find(&self.http, query).await
    }

    /// A cover, downloaded and scaled for the player, with the colours taken from it.
    pub async fn cover(&self, url: &str) -> Result<(SharedPixelBuffer<Rgba8Pixel>, Option<CoverColors>), String> {
        let response = self.http.get(url).send().await.map_err(|e| e.to_string())?;
        let bytes = response.bytes().await.map_err(|e| e.to_string())?;
        tokio::task::spawn_blocking(move || {
            let image = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
            let image = image.resize_to_fill(COVER_SIZE, COVER_SIZE, image::imageops::FilterType::Triangle);
            let colors = settings::cover_colors(image.thumbnail_exact(24, 24).to_rgba8().as_raw());
            let image = image.to_rgba8();
            Ok((SharedPixelBuffer::clone_from_slice(image.as_raw(), image.width(), image.height()), colors))
        })
        .await
        .map_err(|e| e.to_string())?
    }

    /// Stops the engine before exit.
    pub fn shutdown(&self) {
        if let Ok(guard) = self.engine.try_lock()
            && let Some(engine) = guard.as_ref()
        {
            engine.shutdown();
        }
    }
}

fn without_repeats(uris: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    uris.iter().filter(|uri| seen.insert(uri.as_str())).cloned().collect()
}

/// Whether the library is connected, and the Client ID it uses.
pub struct LibraryStatus {
    pub client_id: Option<String>,
    pub connected: bool,
}

pub fn library_status() -> LibraryStatus {
    let login = web_api::load();
    LibraryStatus {
        connected: login.as_ref().is_some_and(WebLogin::has_token),
        client_id: login.map(|l| l.client_id),
    }
}

pub const REDIRECT_URI: &str = web_api::REDIRECT_URI;

const LOGIN_TIMEOUT: Duration = Duration::from_secs(180);
static LOGIN_CANCELLED: AtomicBool = AtomicBool::new(false);

/// Runs a blocking browser login. Gives up after `LOGIN_TIMEOUT` or on
/// `cancel_login`, waking the local callback listener so its port is freed.
async fn browser_flow<T: Send + 'static>(
    abort: fn(),
    login: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    LOGIN_CANCELLED.store(false, Ordering::Relaxed);
    let mut handle = tokio::task::spawn_blocking(login);
    let result = tokio::select! {
        result = &mut handle => result.map_err(|e| e.to_string())?,
        _ = tokio::time::sleep(LOGIN_TIMEOUT) => {
            abort();
            let _ = handle.await;
            return Err("Login timed out. Try again.".into());
        }
    };
    if LOGIN_CANCELLED.load(Ordering::Relaxed) {
        return Err("Login cancelled.".into());
    }
    result
}

/// Stops a login that's waiting for the browser.
pub fn cancel_login() {
    LOGIN_CANCELLED.store(true, Ordering::Relaxed);
    auth::abort_login();
    web_api::abort_login();
}
