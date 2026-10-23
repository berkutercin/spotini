//! The playback engine: a librespot session plus a player. Player events go
//! to `playback`, which keeps the queue moving. The engine also registers as
//! a Spotify Connect device, so the account (and anything that follows it,
//! such as Discord) sees what is playing.

use std::sync::Arc;
use std::time::Duration;

use librespot_connect::{ConnectConfig, LoadRequest, LoadRequestOptions, Spirc};
use librespot_core::{
    Session, SessionConfig, SpotifyUri, authentication::Credentials, config::DeviceType,
};
use librespot_metadata::audio::{AudioItem, UniqueFields};
use librespot_playback::{
    config::{Bitrate, PlayerConfig},
    mixer::{self, Mixer, MixerConfig},
    player::Player,
};
use librespot_protocol::authentication::AuthenticationType;
use serde::Serialize;

use crate::{output, playback};

pub struct Engine {
    session: Session,
    player: Arc<Player>,
    mixer: Arc<dyn Mixer>,
    /// Missing when the Connect device could not be set up; playback then
    /// goes to the player directly.
    spirc: Option<Spirc>,
}

/// What librespot reports about the loaded track.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackInfo {
    pub uri: String,
    pub name: String,
    pub artists: Vec<String>,
    pub album: String,
    pub cover_url: Option<String>,
    pub duration_ms: u32,
}

fn parse_uri(uri: &str) -> Result<SpotifyUri, String> {
    SpotifyUri::from_uri(uri).map_err(|_| format!("Not a Spotify URI: {uri}"))
}

impl Engine {
    /// Connects to Spotify and returns the engine together with reusable
    /// credentials that can be stored for the next launch.
    pub async fn connect(
        device_id: String,
        credentials: Credentials,
        volume: f64,
    ) -> Result<(Self, Credentials), String> {
        let make_mixer = mixer::find(None).ok_or("no software mixer available")?;
        let mixer = make_mixer(MixerConfig::default()).map_err(|e| e.to_string())?;
        let config = SessionConfig {
            device_id,
            // The queue lives in spotini; Connect must not pick tracks itself.
            autoplay: Some(false),
            ..Default::default()
        };

        let mut session = Session::new(config.clone(), None);
        let player = new_player(&session, &mixer);
        let connect_config = ConnectConfig {
            name: "spotini".into(),
            device_type: DeviceType::Computer,
            initial_volume: to_mixer_volume(volume),
            ..Default::default()
        };
        // Spirc connects the session itself.
        let spirc = match Spirc::new(
            connect_config,
            session.clone(),
            credentials.clone(),
            player.clone(),
            mixer.clone(),
        )
        .await
        {
            Ok((spirc, task)) => {
                tokio::spawn(task);
                Some(spirc)
            }
            Err(e) => {
                log::warn!("Spotify Connect unavailable: {e}");
                None
            }
        };
        let player = if spirc.is_some() {
            player
        } else {
            player.stop();
            session.shutdown();
            session = Session::new(config, None);
            session
                .connect(credentials, false)
                .await
                .map_err(|e| e.to_string())?;
            new_player(&session, &mixer)
        };

        let reusable = Credentials {
            username: Some(session.username()),
            auth_type: AuthenticationType::AUTHENTICATION_STORED_SPOTIFY_CREDENTIALS,
            auth_data: session.auth_data(),
        };

        let mut events = player.get_player_event_channel();
        tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                playback::forward_event(event);
            }
        });

        let engine = Self {
            session,
            player,
            mixer,
            spirc,
        };
        engine.set_volume(volume);
        Ok((engine, reusable))
    }

    pub fn session(&self) -> Session {
        self.session.clone()
    }

    pub fn username(&self) -> String {
        self.session.username()
    }

    /// Whether playback goes through the Spotify Connect device.
    pub fn has_connect(&self) -> bool {
        self.spirc.is_some()
    }

    pub fn is_alive(&self) -> bool {
        !self.session.is_invalid() && !self.player.is_invalid()
    }

    pub fn load(&self, uri: &str) -> Result<(), String> {
        let spotify_uri = parse_uri(uri)?;
        if let Some(spirc) = &self.spirc {
            // Commands are ignored until the device is active; activating an
            // already active device only logs a warning.
            let request = LoadRequest::from_tracks(
                vec![uri.to_string()],
                LoadRequestOptions {
                    start_playing: true,
                    ..Default::default()
                },
            );
            if spirc.activate().is_ok() && spirc.load(request).is_ok() {
                return Ok(());
            }
        }
        self.player.load(spotify_uri, true, 0);
        Ok(())
    }

    /// Starts fetching a track so the switch to it is instant.
    pub fn preload(&self, uri: &str) -> Result<(), String> {
        self.player.preload(parse_uri(uri)?);
        Ok(())
    }

    pub fn play(&self) {
        self.player.play();
    }

    pub fn pause(&self) {
        self.player.pause();
    }

    pub fn seek(&self, position_ms: u32) {
        self.player.seek(position_ms);
    }

    /// `volume` is 0.0–1.0.
    pub fn set_volume(&self, volume: f64) {
        let volume = to_mixer_volume(volume);
        self.mixer.set_volume(volume);
        if let Some(spirc) = &self.spirc {
            let _ = spirc.set_volume(volume);
        }
    }

    pub fn shutdown(&self) {
        if let Some(spirc) = &self.spirc {
            let _ = spirc.shutdown();
        }
        self.player.stop();
        self.session.shutdown();
    }
}

/// `volume` is 0.0–1.0.
fn to_mixer_volume(volume: f64) -> u16 {
    (volume.clamp(0.0, 1.0) * u16::MAX as f64).round() as u16
}

fn new_player(session: &Session, mixer: &Arc<dyn Mixer>) -> Arc<Player> {
    let player_config = PlayerConfig {
        bitrate: Bitrate::Bitrate320,
        position_update_interval: Some(Duration::from_secs(1)),
        ..Default::default()
    };
    // The sink is created on the player thread (audio streams aren't Send).
    Player::new(player_config, session.clone(), mixer.get_soft_volume(), output::open)
}

pub fn track_info(item: &AudioItem) -> TrackInfo {
    let (artists, album) = match &item.unique_fields {
        UniqueFields::Track { artists, album, .. } => (
            artists.0.iter().map(|a| a.name.clone()).collect(),
            album.clone(),
        ),
        UniqueFields::Episode { show_name, .. } => (vec![show_name.clone()], String::new()),
        UniqueFields::Local { artists, album, .. } => (
            artists.iter().cloned().collect(),
            album.clone().unwrap_or_default(),
        ),
    };
    let cover_url = item
        .covers
        .iter()
        .max_by_key(|c| c.width)
        .map(|c| c.url.clone());

    TrackInfo {
        uri: item.uri.clone(),
        name: item.name.clone(),
        artists,
        album,
        cover_url,
        duration_ms: item.duration_ms,
    }
}
