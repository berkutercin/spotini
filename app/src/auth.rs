//! Spotify login for the playback session, and storage of the resulting
//! reusable credentials in the Windows Credential Manager.

use keyring::Entry;
use librespot_core::{SessionConfig, authentication::Credentials};
use librespot_oauth::OAuthClientBuilder;
use serde::{Deserialize, Serialize};

// librespot sessions only accept tokens issued to Spotify's own client ID.
const SPOTIFY_CLIENT_ID: &str = "65b708073fc0480ea92a077233ca87bd";
const SPOTIFY_REDIRECT_URI: &str = "http://127.0.0.1:8898/login";

const KEYRING_SERVICE: &str = "spotini";
const KEYRING_USER: &str = "playback-session";

pub const LOGIN_DONE_PAGE: &str = r#"<!doctype html>
<html><head><meta charset="utf-8"><title>spotini</title></head>
<body style="background:#121212;color:#eee;font-family:system-ui;display:grid;place-items:center;height:100vh;margin:0">
<div style="text-align:center"><h1>You're logged in</h1><p>You can close this tab and go back to spotini.</p></div>
</body></html>"#;

#[derive(Serialize, Deserialize)]
pub struct StoredLogin {
    pub device_id: String,
    pub credentials: Credentials,
}

fn entry() -> Result<Entry, String> {
    Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|e| e.to_string())
}

pub fn load() -> Option<StoredLogin> {
    let json = entry().ok()?.get_password().ok()?;
    serde_json::from_str(&json).ok()
}

pub fn save(login: &StoredLogin) -> Result<(), String> {
    let json = serde_json::to_string(login).map_err(|e| e.to_string())?;
    entry()?.set_password(&json).map_err(|e| e.to_string())
}

pub fn clear() {
    if let Ok(entry) = entry() {
        let _ = entry.delete_credential();
    }
}

/// Reuses the stored device ID so Spotify sees the same device across logins.
pub fn device_id() -> String {
    load()
        .map(|login| login.device_id)
        .unwrap_or_else(|| SessionConfig::default().device_id)
}

/// Wakes a login that's waiting for the browser on `redirect_uri`'s port: the
/// listener gets a request with no code, fails, and frees the port.
pub fn abort_listener(redirect_uri: &str) {
    let Some(port) = redirect_uri.rsplit(':').next().and_then(|rest| rest.split('/').next()) else {
        return;
    };
    if let Ok(mut stream) = std::net::TcpStream::connect(format!("127.0.0.1:{port}")) {
        use std::io::Write;
        let _ = stream.write_all(b"GET /?error=cancelled HTTP/1.1\r\n\r\n");
    }
}

pub fn abort_login() {
    abort_listener(SPOTIFY_REDIRECT_URI);
}

#[cfg(test)]
mod tests {
    use super::abort_listener;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    #[test]
    fn abort_wakes_a_waiting_listener() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let waiter = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(stream).read_line(&mut line).unwrap();
            line
        });
        abort_listener(&format!("http://127.0.0.1:{port}/login"));
        assert!(waiter.join().unwrap().starts_with("GET /?error=cancelled"));
    }
}

/// Opens the browser for Spotify's consent page and blocks until the redirect
/// arrives on the local listener (or `abort_login` wakes it).
pub fn browser_login() -> Result<Credentials, String> {
    let client = OAuthClientBuilder::new(SPOTIFY_CLIENT_ID, SPOTIFY_REDIRECT_URI, vec!["streaming"])
        .open_in_browser()
        .with_custom_message(LOGIN_DONE_PAGE)
        .build()
        .map_err(|e| e.to_string())?;
    let token = client.get_access_token().map_err(|e| e.to_string())?;
    Ok(Credentials::with_access_token(token.access_token))
}
