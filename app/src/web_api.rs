//! Spotify Web API access through the user's own developer app (PKCE, no
//! client secret). The client ID and tokens live in the Windows Credential
//! Manager.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use keyring::Entry;
use librespot_oauth::{OAuthClient, OAuthClientBuilder, OAuthToken};
use reqwest::{Method, StatusCode, header::CONTENT_LENGTH};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::auth::LOGIN_DONE_PAGE;

pub const REDIRECT_URI: &str = "http://127.0.0.1:8899/callback";
const SCOPES: &[&str] = &[
    "playlist-read-private",
    "playlist-read-collaborative",
    "user-library-read",
    "user-library-modify",
    "playlist-modify-public",
    "playlist-modify-private",
    "user-read-playback-state",
];
const API_BASE: &str = "https://api.spotify.com/v1";

const KEYRING_SERVICE: &str = "spotini";
const KEYRING_USER: &str = "web-api";

#[derive(Serialize, Deserialize, Clone)]
struct Token {
    access_token: String,
    refresh_token: String,
    /// Unix seconds.
    expires_at: u64,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct WebLogin {
    pub client_id: String,
    token: Option<Token>,
}

impl WebLogin {
    pub fn new(client_id: String) -> Self {
        Self {
            client_id,
            token: None,
        }
    }

    pub fn has_token(&self) -> bool {
        self.token.is_some()
    }
}

fn entry() -> Result<Entry, String> {
    Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|e| e.to_string())
}

pub fn load() -> Option<WebLogin> {
    let json = entry().ok()?.get_password().ok()?;
    serde_json::from_str(&json).ok()
}

pub fn save(login: &WebLogin) -> Result<(), String> {
    let json = serde_json::to_string(login).map_err(|e| e.to_string())?;
    entry()?.set_password(&json).map_err(|e| e.to_string())
}

pub fn clear() {
    if let Ok(entry) = entry() {
        let _ = entry.delete_credential();
    }
}

/// Builds an API URL with properly encoded query parameters.
pub fn api_url(path: &str, params: &[(&str, &str)]) -> String {
    let mut url = reqwest::Url::parse(&format!("{API_BASE}{path}")).expect("valid API URL");
    url.query_pairs_mut().extend_pairs(params);
    url.into()
}

pub fn is_valid_client_id(client_id: &str) -> bool {
    client_id.len() == 32 && client_id.chars().all(|c| c.is_ascii_hexdigit())
}

fn oauth_client(client_id: &str) -> Result<OAuthClient, String> {
    OAuthClientBuilder::new(client_id, REDIRECT_URI, SCOPES.to_vec())
        .open_in_browser()
        .with_custom_message(LOGIN_DONE_PAGE)
        .build()
        .map_err(|e| e.to_string())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn to_token(token: OAuthToken) -> Token {
    let expires_in = token.expires_at.saturating_duration_since(Instant::now());
    Token {
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        expires_at: unix_now() + expires_in.as_secs(),
    }
}

pub fn abort_login() {
    crate::auth::abort_listener(REDIRECT_URI);
}

/// Opens the browser for consent and blocks until the redirect arrives.
pub fn browser_login(client_id: String) -> Result<WebLogin, String> {
    let token = oauth_client(&client_id)?
        .get_access_token()
        .map_err(|e| e.to_string())?;
    Ok(WebLogin {
        client_id,
        token: Some(to_token(token)),
    })
}

pub struct WebApi {
    http: reqwest::Client,
    login: Mutex<WebLogin>,
}

impl WebApi {
    pub fn new(login: WebLogin) -> Self {
        Self {
            http: reqwest::Client::new(),
            login: Mutex::new(login),
        }
    }

    async fn access_token(&self, force_refresh: bool) -> Result<String, String> {
        let mut login = self.login.lock().await;
        let token = login.token.as_ref().ok_or("Library not connected")?;
        if !force_refresh && token.expires_at > unix_now() + 60 {
            return Ok(token.access_token.clone());
        }

        let mut refreshed = oauth_client(&login.client_id)?
            .refresh_token_async(&token.refresh_token)
            .await
            .map_err(|e| format!("Could not refresh Spotify login: {e}"))?;
        // Spotify may or may not rotate the refresh token.
        if refreshed.refresh_token.is_empty() {
            refreshed.refresh_token = token.refresh_token.clone();
        }
        let token = to_token(refreshed);
        let access_token = token.access_token.clone();
        login.token = Some(token);
        save(&login)?;
        Ok(access_token)
    }

    /// GETs an API path or a full `next` URL.
    pub async fn get(&self, path_or_url: &str) -> Result<Value, String> {
        self.request(Method::GET, path_or_url).await
    }

    pub async fn request(&self, method: Method, path_or_url: &str) -> Result<Value, String> {
        self.send(method, path_or_url, None).await
    }

    /// Sends a request with a JSON body.
    pub async fn request_json(&self, method: Method, path_or_url: &str, body: &Value) -> Result<Value, String> {
        self.send(method, path_or_url, Some(body)).await
    }

    /// Sends a request, refreshing the token once on 401 and waiting once on
    /// 429. An empty response body comes back as `Value::Null`.
    async fn send(&self, method: Method, path_or_url: &str, body: Option<&Value>) -> Result<Value, String> {
        let url = if path_or_url.starts_with("https://") {
            path_or_url.to_string()
        } else {
            format!("{API_BASE}{path_or_url}")
        };

        let mut force_refresh = false;
        let mut retried = false;
        loop {
            let token = self.access_token(force_refresh).await?;
            let mut request = self.http.request(method.clone(), &url).bearer_auth(token);
            if let Some(body) = body {
                request = request.json(body);
            } else if method != Method::GET {
                // Spotify's front end refuses bodiless writes without a length.
                request = request.header(CONTENT_LENGTH, 0);
            }
            let response = request
                .send()
                .await
                .map_err(|e| e.to_string())?;

            match response.status() {
                StatusCode::UNAUTHORIZED if !retried => force_refresh = true,
                StatusCode::TOO_MANY_REQUESTS if !retried => {
                    let wait = response
                        .headers()
                        .get("retry-after")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.parse::<u64>().ok())
                        .unwrap_or(2)
                        .min(10);
                    tokio::time::sleep(Duration::from_secs(wait)).await;
                }
                status if status.is_success() => {
                    let body = response.text().await.map_err(|e| e.to_string())?;
                    if body.trim().is_empty() {
                        return Ok(Value::Null);
                    }
                    return serde_json::from_str(&body).map_err(|e| e.to_string());
                }
                StatusCode::FORBIDDEN if method != Method::GET => {
                    // Usually a permission the login predates (playlist editing came later).
                    let body = response.text().await.unwrap_or_default();
                    log::warn!("Spotify refused {method} {url}: {body}");
                    return Err(
                        "Spotify refused this. Reconnect the library to allow it: settings → change client id → Continue."
                            .into(),
                    );
                }
                status => {
                    let body = response.text().await.unwrap_or_default();
                    return Err(format!("Spotify API error {status}: {body}"));
                }
            }
            retried = true;
        }
    }

    /// Collects `items` across all pages of a paging object.
    pub async fn get_all(&self, path: &str) -> Result<Vec<Value>, String> {
        let mut items = Vec::new();
        let mut next = Some(path.to_string());
        while let Some(url) = next {
            let mut page = self.get(&url).await?;
            if let Some(Value::Array(page_items)) = page.get_mut("items").map(Value::take) {
                items.extend(page_items);
            }
            next = page["next"].as_str().map(str::to_string);
        }
        Ok(items)
    }
}
