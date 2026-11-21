//! Spotify (remote control).
//!
//! Connected on the server, never from the app: `plate authorize spotify` runs the sign-in
//! on your own computer and stores the app credentials and tokens in the database,
//! encrypted with TOKEN_KEY. They are never sent to the app or to agents, and Spotify is not
//! reachable from the agent link: its terms forbid feeding its content to a model.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::{DateTime, Duration, Utc};
use gym_core::api::{Connections, MusicCommand, MusicDevice, NowPlaying, Playlist, ProviderStatus};
use serde::{Deserialize, Serialize};

use crate::auth::Owner;
use crate::config::ProviderApp;
use crate::crypto::{open, seal};
use crate::error::{AppError, AppResult, bad};
use crate::state::AppState;
use crate::store;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Spotify,
}

impl Provider {
    pub fn key(self) -> &'static str {
        match self {
            Provider::Spotify => "spotify",
        }
    }

    pub fn parse(s: &str) -> Option<Provider> {
        match s {
            "spotify" => Some(Provider::Spotify),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Provider::Spotify => "Spotify",
        }
    }

    /// Where the provider sends the browser back to during `plate authorize`.
    pub fn local_redirect(self) -> &'static str {
        match self {
            // Spotify needs this exact address in the app's Redirect URIs (loopback, not "localhost").
            Provider::Spotify => "http://127.0.0.1:8765/callback",
        }
    }

    pub fn authorize_url(self, client_id: &str, state: &str) -> String {
        let (base, scope) = match self {
            Provider::Spotify => ("https://accounts.spotify.com/authorize", "user-read-playback-state user-modify-playback-state user-read-currently-playing playlist-read-private"),
        };
        let params = [("client_id", client_id), ("redirect_uri", self.local_redirect()), ("response_type", "code"), ("scope", scope), ("state", state)];
        url::Url::parse_with_params(base, &params).map(|u| u.to_string()).unwrap_or_default()
    }
}

/// Everything needed to call a provider, sealed together in one row.
#[derive(Serialize, Deserialize, Clone)]
pub struct Tokens {
    pub access: String,
    pub refresh: String,
    pub expires_at: DateTime<Utc>,
    /// The developer app's id and secret, so the server needs no extra settings.
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub client_secret: Option<String>,
}

fn key(st: &AppState) -> AppResult<&[u8; 32]> {
    st.cfg.token_key.as_ref().ok_or_else(|| AppError::Unavailable("TOKEN_KEY is not set, so the server cannot store sign-ins safely".into()))
}

async fn save_tokens(st: &AppState, p: Provider, t: &Tokens, account: Option<&str>, fresh: bool) -> AppResult<()> {
    store_tokens(&st.db, key(st)?, p, t, account, fresh).await
}

/// Seal and store a provider's tokens. Used by the server and by `plate authorize`.
pub async fn store_tokens(db: &sqlx::PgPool, token_key: &[u8; 32], p: Provider, t: &Tokens, account: Option<&str>, fresh: bool) -> AppResult<()> {
    let sealed = seal(token_key, &serde_json::to_vec(t)?)?;
    sqlx::query(
        "insert into provider_tokens (provider, ciphertext, nonce, account, expires_at, connected_at, updated_at) values ($1, $2, $3, $4, $5, now(), now())
         on conflict (provider) do update set ciphertext = excluded.ciphertext, nonce = excluded.nonce, account = coalesce(excluded.account, provider_tokens.account),
           expires_at = excluded.expires_at, updated_at = now(), connected_at = case when $6 then now() else provider_tokens.connected_at end",
    )
    .bind(p.key())
    .bind(&sealed.ciphertext)
    .bind(&sealed.nonce)
    .bind(account)
    .bind(t.expires_at)
    .bind(fresh)
    .execute(db)
    .await?;
    Ok(())
}

async fn load_tokens(st: &AppState, p: Provider) -> AppResult<Option<Tokens>> {
    let row: Option<(Vec<u8>, Vec<u8>)> = sqlx::query_as("select ciphertext, nonce from provider_tokens where provider = $1").bind(p.key()).fetch_optional(&st.db).await?;
    match row {
        Some((ct, nonce)) => Ok(Some(serde_json::from_slice(&open(key(st)?, &ct, &nonce)?)?)),
        None => Ok(None),
    }
}

#[derive(Deserialize)]
pub struct TokenReply {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<i64>,
    #[serde(default)]
    pub expires_at: Option<i64>,
}

impl TokenReply {
    pub fn expiry(&self) -> DateTime<Utc> {
        match (self.expires_at, self.expires_in) {
            (Some(at), _) => DateTime::from_timestamp(at, 0).unwrap_or_else(Utc::now),
            (None, Some(secs)) => Utc::now() + Duration::seconds(secs),
            _ => Utc::now() + Duration::minutes(50),
        }
    }
}

/// The developer app: from the stored row, else from the environment.
fn app_for(st: &AppState, p: Provider, t: &Tokens) -> AppResult<ProviderApp> {
    if let (Some(id), Some(secret)) = (&t.client_id, &t.client_secret) {
        return Ok(ProviderApp { client_id: id.clone(), client_secret: secret.clone() });
    }
    let env = match p {
        Provider::Spotify => st.cfg.spotify.clone(),
    };
    env.ok_or_else(|| AppError::Unavailable(format!("{} has no app id and secret. Run plate authorize {} again", p.label(), p.key())))
}

pub async fn token_request(http: &reqwest::Client, p: Provider, app: &ProviderApp, form: &[(&str, &str)]) -> AppResult<TokenReply> {
    let req = match p {
        Provider::Spotify => http.post("https://accounts.spotify.com/api/token").basic_auth(&app.client_id, Some(&app.client_secret)).form(form),
    };
    let res = req.send().await.map_err(|e| AppError::Upstream(format!("{} did not answer: {e}", p.label())))?;
    if !res.status().is_success() {
        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        tracing::warn!(provider = p.key(), %status, body = %text, "token request failed");
        return Err(AppError::Upstream(format!("{} refused the sign-in ({status})", p.label())));
    }
    res.json().await.map_err(|e| AppError::Upstream(format!("{} sent an unexpected reply: {e}", p.label())))
}

/// A working access token, refreshed when it is about to expire.
async fn access(st: &AppState, p: Provider) -> AppResult<String> {
    let t = load_tokens(st, p).await?.ok_or_else(|| AppError::Conflict(format!("{} is not connected", p.label())))?;
    if t.expires_at > Utc::now() + Duration::minutes(2) {
        return Ok(t.access);
    }
    let app = app_for(st, p, &t)?;
    let r = token_request(&st.http, p, &app, &[("grant_type", "refresh_token"), ("refresh_token", &t.refresh)]).await?;
    let fresh = Tokens { access: r.access_token.clone(), refresh: r.refresh_token.clone().unwrap_or(t.refresh), expires_at: r.expiry(), client_id: t.client_id, client_secret: t.client_secret };
    save_tokens(st, p, &fresh, None, false).await?;
    Ok(fresh.access)
}

pub async fn connections(st: &AppState) -> AppResult<Connections> {
    let rows: Vec<(String, Option<String>, DateTime<Utc>)> = sqlx::query_as("select provider, account, connected_at from provider_tokens").fetch_all(&st.db).await?;
    let status = |p: Provider, configured: bool| {
        let row = rows.iter().find(|r| r.0 == p.key());
        let detail = match (p, row) {
            // Spotify refresh tokens stop working six months after sign-in.
            (Provider::Spotify, Some((_, _, at))) if Utc::now() - *at > Duration::days(165) => Some("Run plate authorize spotify again soon: Spotify sign-ins last six months".into()),
            (Provider::Spotify, Some(_)) => Some("Needs Spotify Premium and the Spotify app opened once".into()),
            _ => None,
        };
        ProviderStatus { configured: configured || row.is_some(), connected: row.is_some() && st.cfg.token_key.is_some(), account: row.and_then(|r| r.1.clone()), detail }
    };
    let mut c = st.db.acquire().await?;
    let key_set: Option<String> = store::get_setting(&mut c, "ingest_key_hash").await?;
    Ok(Connections {
        health_last_delivery: store::health_last_delivery(&mut c).await?,
        health_key_set: key_set.is_some(),
        spotify: status(Provider::Spotify, st.cfg.spotify.is_some()),
    })
}

pub async fn connections_route(_: Owner, State(st): State<AppState>) -> AppResult<Json<Connections>> {
    Ok(Json(connections(&st).await?))
}

// ---------- Spotify ----------

async fn spotify(st: &AppState, method: reqwest::Method, path: &str, body: Option<serde_json::Value>) -> AppResult<reqwest::Response> {
    let token = access(st, Provider::Spotify).await?;
    let mut req = st.http.request(method, format!("https://api.spotify.com/v1{path}")).bearer_auth(token);
    if let Some(b) = body {
        req = req.json(&b);
    } else {
        req = req.header("content-length", "0");
    }
    req.send().await.map_err(|e| AppError::Upstream(format!("Spotify did not answer: {e}")))
}

pub async fn now_playing(_: Owner, State(st): State<AppState>) -> AppResult<Json<NowPlaying>> {
    let res = spotify(&st, reqwest::Method::GET, "/me/player", None).await?;
    if res.status() == StatusCode::NO_CONTENT {
        return Ok(Json(NowPlaying { playing: false, track: None, artist: None, album_art: None, track_url: None, device: None, no_device: true }));
    }
    if !res.status().is_success() {
        return Err(AppError::Upstream(format!("Spotify said {}", res.status())));
    }
    let v: serde_json::Value = res.json().await.map_err(|e| AppError::Upstream(e.to_string()))?;
    let item = &v["item"];
    Ok(Json(NowPlaying {
        playing: v["is_playing"].as_bool().unwrap_or(false),
        track: item["name"].as_str().map(str::to_string),
        artist: item["artists"].as_array().map(|a| a.iter().filter_map(|x| x["name"].as_str()).collect::<Vec<_>>().join(", ")).filter(|s| !s.is_empty()),
        album_art: item["album"]["images"].as_array().and_then(|i| i.iter().rev().find(|x| x["width"].as_u64().unwrap_or(0) >= 200).or(i.first())).and_then(|x| x["url"].as_str()).map(str::to_string),
        track_url: item["external_urls"]["spotify"].as_str().map(str::to_string),
        device: v["device"]["name"].as_str().map(str::to_string),
        no_device: false,
    }))
}

pub async fn playlists(_: Owner, State(st): State<AppState>) -> AppResult<Json<Vec<Playlist>>> {
    let res = spotify(&st, reqwest::Method::GET, "/me/playlists?limit=30", None).await?;
    if !res.status().is_success() {
        return Err(AppError::Upstream(format!("Spotify said {}", res.status())));
    }
    let v: serde_json::Value = res.json().await.map_err(|e| AppError::Upstream(e.to_string()))?;
    Ok(Json(
        v["items"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|p| {
                        Some(Playlist {
                            id: p["id"].as_str()?.to_string(),
                            name: p["name"].as_str().unwrap_or("Playlist").to_string(),
                            image: p["images"].as_array().and_then(|i| i.first()).and_then(|i| i["url"].as_str()).map(str::to_string),
                            url: p["external_urls"]["spotify"].as_str().unwrap_or("https://open.spotify.com").to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
    ))
}

async fn device_list(st: &AppState) -> AppResult<Vec<MusicDevice>> {
    let res = spotify(st, reqwest::Method::GET, "/me/player/devices", None).await?;
    if !res.status().is_success() {
        return Err(AppError::Upstream(format!("Spotify said {}", res.status())));
    }
    let v: serde_json::Value = res.json().await.map_err(|e| AppError::Upstream(e.to_string()))?;
    Ok(v["devices"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter(|d| d["is_restricted"] != true)
                .filter_map(|d| {
                    Some(MusicDevice {
                        id: d["id"].as_str()?.to_string(),
                        name: d["name"].as_str().unwrap_or("Spotify").to_string(),
                        kind: d["type"].as_str().unwrap_or("Device").to_string(),
                        active: d["is_active"].as_bool().unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default())
}

/// Every Spotify app that is open: the phone, the desktop app, a browser on open.spotify.com.
pub async fn devices(_: Owner, State(st): State<AppState>) -> AppResult<Json<Vec<MusicDevice>>> {
    Ok(Json(device_list(&st).await?))
}

fn valid_device(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_alphanumeric())
}

/// Play, pause, skip, start a playlist, or move playback to another device. Works with
/// any Spotify app: the phone, the desktop app or open.spotify.com. With no device chosen
/// and none playing, the one already active wins, then a computer, then a phone.
pub async fn command(_: Owner, State(st): State<AppState>, Json(cmd): Json<MusicCommand>) -> AppResult<StatusCode> {
    let device = cmd.device_id.clone().filter(|d| valid_device(d));
    if cmd.device_id.is_some() && device.is_none() {
        return Err(bad("Pick a device"));
    }
    if cmd.action == "transfer" {
        let id = device.ok_or_else(|| bad("Pick a device"))?;
        let res = spotify(&st, reqwest::Method::PUT, "/me/player", Some(serde_json::json!({ "device_ids": [id], "play": true }))).await?;
        return spotify_result(res);
    }
    let (method, path, body) = match cmd.action.as_str() {
        "play" => (reqwest::Method::PUT, "/me/player/play".to_string(), None),
        "pause" => (reqwest::Method::PUT, "/me/player/pause".to_string(), None),
        "next" => (reqwest::Method::POST, "/me/player/next".to_string(), None),
        "previous" => (reqwest::Method::POST, "/me/player/previous".to_string(), None),
        "playlist" => {
            let id = cmd.playlist_id.as_deref().filter(|id| id.chars().all(|c| c.is_ascii_alphanumeric())).ok_or_else(|| bad("Pick a playlist"))?;
            (reqwest::Method::PUT, "/me/player/play".to_string(), Some(serde_json::json!({ "context_uri": format!("spotify:playlist:{id}") })))
        }
        _ => return Err(bad("Unknown music action")),
    };
    let with = |d: &str| format!("{path}?device_id={d}");
    let first = match &device {
        Some(d) => with(d),
        None => path.clone(),
    };
    let res = spotify(&st, method.clone(), &first, body.clone()).await?;
    if res.status() == StatusCode::NOT_FOUND && device.is_none() {
        // NO_ACTIVE_DEVICE: pick one and retry.
        let list = device_list(&st).await?;
        let pick = list
            .iter()
            .find(|d| d.active)
            .or_else(|| list.iter().find(|d| d.kind == "Computer"))
            .or_else(|| list.iter().find(|d| d.kind == "Smartphone"))
            .or(list.first())
            .map(|d| d.id.clone());
        let Some(device) = pick else {
            return Err(AppError::Conflict("No Spotify app is open. Open Spotify on this computer, at open.spotify.com, or on your phone, then try again".into()));
        };
        return spotify_result(spotify(&st, method, &with(&device), body).await?);
    }
    spotify_result(res)
}

fn spotify_result(res: reqwest::Response) -> AppResult<StatusCode> {
    match res.status() {
        s if s.is_success() => Ok(StatusCode::NO_CONTENT),
        StatusCode::FORBIDDEN => Err(AppError::Conflict("Spotify refused: remote control needs Spotify Premium".into())),
        StatusCode::NOT_FOUND => Err(AppError::Conflict("That Spotify app is not open any more. Open it and try again".into())),
        s => Err(AppError::Upstream(format!("Spotify said {s}"))),
    }
}

