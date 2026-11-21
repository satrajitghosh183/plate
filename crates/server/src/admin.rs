//! Everything that connects something to Plate, done on the server side.
//!
//! The app has no screens for keys or sign-ins. These functions back the `plate`
//! command (see `main.rs`), which you run on your own computer with `scripts/plate.sh`:
//!
//! - `phone-key`: the write-only key the iPhone's Shortcuts use for Apple Health (and
//!   Lifesum, which arrives through Apple Health).
//! - `agent add | list | revoke`: links for Muse and Claude.
//! - `authorize strava | spotify`: signs in through your browser and stores the tokens,
//!   encrypted, straight in the database.

use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

use crate::config::{Config, ProviderApp};
use crate::crypto::{constant_time_eq, new_secret, sha256_hex};
use crate::providers::{Provider, Tokens, store_tokens, token_request};

pub struct PhoneKey {
    pub key: String,
    pub workout_ended_url: String,
    pub health_url: String,
}

/// How long the previous phone key keeps working after a new one is made, so the
/// shortcuts can be updated one at a time. `plate phone-key --now` skips the overlap.
pub const PHONE_KEY_OVERLAP_HOURS: i64 = 24;

/// Make a new phone key. The old one keeps working for `overlap_hours` (0: stops at once).
/// Only hashes are stored; the key itself is shown once and never kept or logged.
pub async fn new_phone_key_with_overlap(db: &PgPool, cfg: &Config, overlap_hours: i64) -> anyhow::Result<PhoneKey> {
    let key = new_secret("phone");
    let mut tx = db.begin().await?;
    let old: Option<String> = crate::store::get_setting(&mut tx, "ingest_key_hash").await.map_err(|e| anyhow!(e.message()))?;
    match old {
        Some(h) if overlap_hours > 0 => {
            let until = Utc::now() + chrono::Duration::hours(overlap_hours);
            crate::store::set_setting(&mut tx, "ingest_key_prev", &serde_json::json!({"hash": h, "until": until})).await.map_err(|e| anyhow!(e.message()))?;
        }
        _ => {
            sqlx::query("delete from settings where key = 'ingest_key_prev'").execute(&mut *tx).await?;
        }
    }
    crate::store::set_setting(&mut tx, "ingest_key_hash", &sha256_hex(&key)).await.map_err(|e| anyhow!(e.message()))?;
    tx.commit().await?;
    Ok(PhoneKey { key, workout_ended_url: cfg.public_link("/ingest/workout-ended"), health_url: cfg.public_link("/ingest/health") })
}

/// Make a new phone key; the old one stops working at once.
pub async fn new_phone_key(db: &PgPool, cfg: &Config) -> anyhow::Result<PhoneKey> {
    new_phone_key_with_overlap(db, cfg, 0).await
}

pub struct NewAgent {
    pub id: Uuid,
    pub name: String,
    pub key: String,
    /// The link to give the agent, with the key in it.
    pub link: String,
    /// The address for clients that send the key in a header instead.
    pub endpoint: String,
}

pub async fn add_agent(db: &PgPool, cfg: &Config, name: &str) -> anyhow::Result<NewAgent> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 40 {
        bail!("Give the agent a name of up to 40 characters");
    }
    let taken: Option<(Uuid,)> = sqlx::query_as("select id from agents where lower(name) = lower($1) and revoked_at is null").bind(name).fetch_optional(db).await?;
    if taken.is_some() {
        bail!("An agent called {name} already has a link. Revoke it first to make a new one");
    }
    let key = new_secret("agent");
    let id = Uuid::new_v4();
    sqlx::query("insert into agents (id, name, key_hash, key_prefix) values ($1, $2, $3, $4)")
        .bind(id)
        .bind(name)
        .bind(sha256_hex(&key))
        .bind(key.chars().take(10).collect::<String>())
        .execute(db)
        .await?;
    let endpoint = cfg.public_link("/mcp");
    Ok(NewAgent { id, name: name.to_string(), link: format!("{endpoint}/{key}"), endpoint, key })
}

pub struct AgentRow {
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked: bool,
    pub ask_first: bool,
}

pub async fn list_agents(db: &PgPool) -> anyhow::Result<Vec<AgentRow>> {
    type Row = (String, DateTime<Utc>, Option<DateTime<Utc>>, Option<DateTime<Utc>>, bool);
    let rows: Vec<Row> =
        sqlx::query_as("select name, created_at, last_used_at, revoked_at, ask_first from agents order by created_at").fetch_all(db).await?;
    Ok(rows.into_iter().map(|(name, created_at, last_used_at, revoked_at, ask_first)| AgentRow { name, created_at, last_used_at, revoked: revoked_at.is_some(), ask_first }).collect())
}

pub async fn revoke_agent(db: &PgPool, name: &str) -> anyhow::Result<u64> {
    let done = sqlx::query("update agents set revoked_at = now() where lower(name) = lower($1) and revoked_at is null").bind(name.trim()).execute(db).await?;
    Ok(done.rows_affected())
}

fn env(name: &str) -> anyhow::Result<String> {
    std::env::var(name).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty()).with_context(|| format!("{name} is not set. Add it to your .env"))
}

/// Sign in to Strava or Spotify in your browser and store the tokens in the database.
///
/// Reads the developer app's id and secret from `STRAVA_CLIENT_ID` / `STRAVA_CLIENT_SECRET`
/// (or the `SPOTIFY_` pair) in your `.env`, and seals them together with the tokens, so the
/// server needs nothing else. Returns the account name the provider reported.
pub async fn authorize(db: &PgPool, cfg: &Config, p: Provider) -> anyhow::Result<String> {
    let token_key = cfg.token_key.context("TOKEN_KEY is not set. It must match the one on Render")?;
    let prefix = p.key().to_ascii_uppercase();
    let app = ProviderApp { client_id: env(&format!("{prefix}_CLIENT_ID"))?, client_secret: env(&format!("{prefix}_CLIENT_SECRET"))? };
    let state = new_secret("state");
    let url = p.authorize_url(&app.client_id, &state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:8765").await.context("Port 8765 is busy. Close whatever uses it and try again")?;
    println!("Opening {} in your browser. Approve Plate there.", p.label());
    println!("If nothing opens, visit:\n{url}\n");
    let _ = std::process::Command::new("open").arg(&url).status();

    let code = tokio::time::timeout(Duration::from_secs(300), wait_for_code(&listener, &state)).await.map_err(|_| anyhow!("No answer from the browser within five minutes"))??;
    let http = reqwest::Client::builder().timeout(Duration::from_secs(20)).build()?;
    let r = token_request(&http, p, &app, &[("grant_type", "authorization_code"), ("code", &code), ("redirect_uri", p.local_redirect())]).await.map_err(|e| anyhow!(e.message()))?;
    let refresh = r.refresh_token.clone().with_context(|| format!("{} sent no refresh token", p.label()))?;
    let account = match p {
        Provider::Strava => r.athlete.as_ref().map(|a| format!("{} {}", a["firstname"].as_str().unwrap_or(""), a["lastname"].as_str().unwrap_or("")).trim().to_string()),
        Provider::Spotify => spotify_name(&http, &r.access_token).await,
    };
    let tokens = Tokens { access: r.access_token.clone(), refresh, expires_at: r.expiry(), client_id: Some(app.client_id), client_secret: Some(app.client_secret) };
    store_tokens(db, &token_key, p, &tokens, account.as_deref(), true).await.map_err(|e| anyhow!(e.message()))?;
    Ok(account.unwrap_or_else(|| "your account".into()))
}

async fn spotify_name(http: &reqwest::Client, token: &str) -> Option<String> {
    let res = http.get("https://api.spotify.com/v1/me").bearer_auth(token).send().await.ok()?.error_for_status().ok()?;
    let me: serde_json::Value = res.json().await.ok()?;
    me["display_name"].as_str().map(str::to_string)
}

/// Take browser requests on 127.0.0.1:8765 until the provider's callback arrives.
async fn wait_for_code(listener: &tokio::net::TcpListener, state: &str) -> anyhow::Result<String> {
    loop {
        let (mut sock, _) = listener.accept().await?;
        let mut buf = vec![0u8; 8192];
        let n = sock.read(&mut buf).await.unwrap_or(0);
        let head = String::from_utf8_lossy(&buf[..n]);
        let target = head.lines().next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("/").to_string();
        let parsed = url::Url::parse(&format!("http://127.0.0.1:8765{target}")).ok();
        let Some(u) = parsed.filter(|u| u.path() == "/callback") else {
            let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").await;
            continue;
        };
        let q = |k: &str| u.query_pairs().find(|(name, _)| name == k).map(|(_, v)| v.into_owned());
        let (body, outcome) = match (q("error"), q("state"), q("code")) {
            (Some(e), _, _) => ("Sign-in was cancelled. You can close this tab.", Err(anyhow!("The sign-in was cancelled ({e})"))),
            (None, Some(s), Some(code)) if constant_time_eq(&s, state) => ("Plate is connected. You can close this tab.", Ok(code)),
            _ => ("That reply didn't match this sign-in. You can close this tab.", Err(anyhow!("The reply's state did not match; try again"))),
        };
        let page = format!(
            "<!doctype html><meta charset=utf-8><title>Plate</title><body style=\"background:#000;color:#f5f5f7;font:17px -apple-system,sans-serif;display:grid;place-items:center;height:90vh\"><p>{body}</p>"
        );
        let _ = sock
            .write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{page}", page.len()).as_bytes())
            .await;
        return outcome;
    }
}

// ---------- ways to sign in ----------

/// Your email: the only one that password and Google sign-in accept.
pub async fn set_owner_email(db: &PgPool, email: &str) -> anyhow::Result<String> {
    let email = email.trim().to_lowercase();
    if !email.contains('@') || email.len() > 200 {
        bail!("That doesn't look like an email address");
    }
    let mut c = db.acquire().await?;
    crate::store::set_setting(&mut c, "owner_email", &email).await.map_err(|e| anyhow!(e.message()))?;
    Ok(email)
}

pub async fn set_password(db: &PgPool, password: &str) -> anyhow::Result<()> {
    if password.chars().count() < 12 {
        bail!("Use at least 12 characters");
    }
    let hash = crate::auth::hash_password(password)?;
    let mut c = db.acquire().await?;
    crate::store::set_setting(&mut c, "password_hash", &hash).await.map_err(|e| anyhow!(e.message()))
}

/// Google sign-in: reads GOOGLE_CLIENT_ID and GOOGLE_CLIENT_SECRET from your .env and stores them sealed.
pub async fn set_google(db: &PgPool, cfg: &Config) -> anyhow::Result<String> {
    let key = cfg.token_key.context("TOKEN_KEY is not set. It must match the one on Render")?;
    crate::auth::save_google_app(db, &key, &env("GOOGLE_CLIENT_ID")?, &env("GOOGLE_CLIENT_SECRET")?).await?;
    Ok(cfg.app_link("/api/auth/google/callback"))
}

/// Switch password or Google sign-in off. Passkeys stay.
pub async fn sign_in_off(db: &PgPool, which: &str) -> anyhow::Result<()> {
    let key = match which {
        "password" => "password_hash",
        "google" => "google_app",
        _ => bail!("Use password or google"),
    };
    sqlx::query("delete from settings where key = $1").bind(key).execute(db).await?;
    Ok(())
}

/// Sign out every device.
pub async fn sign_out_everywhere(db: &PgPool) -> anyhow::Result<u64> {
    Ok(sqlx::query("delete from auth_sessions").execute(db).await?.rows_affected())
}

/// One line per connection, for `plate status`.
pub async fn status(db: &PgPool) -> anyhow::Result<Vec<String>> {
    let mut out = Vec::new();
    let has = |k: &'static str| async move { sqlx::query_as::<_, (i64,)>("select count(*) from settings where key = $1").bind(k).fetch_one(db).await.map(|r| r.0 > 0) };
    let email: Option<(serde_json::Value,)> = sqlx::query_as("select value from settings where key = 'owner_email'").fetch_optional(db).await?;
    let passkeys: (i64,) = sqlx::query_as("select count(*) from passkeys").fetch_one(db).await?;
    let sessions: (i64,) = sqlx::query_as("select count(*) from auth_sessions where expires_at > now()").fetch_one(db).await?;
    out.push(format!(
        "Sign-in: {} passkey(s); password {}; Google {}; email {}; {} device(s) signed in",
        passkeys.0,
        if has("password_hash").await? { "on" } else { "off" },
        if has("google_app").await? { "on" } else { "off" },
        email.and_then(|e| e.0.as_str().map(str::to_string)).unwrap_or_else(|| "not set".into()),
        sessions.0
    ));
    let key: Option<(serde_json::Value,)> = sqlx::query_as("select value from settings where key = 'ingest_key_hash'").fetch_optional(db).await?;
    let prev: Option<(serde_json::Value,)> = sqlx::query_as("select value from settings where key = 'ingest_key_prev'").fetch_optional(db).await?;
    let prev_until = prev.and_then(|p| p.0.get("until").and_then(|u| u.as_str()).and_then(|u| u.parse::<DateTime<Utc>>().ok())).filter(|t| *t > Utc::now());
    let last: (Option<DateTime<Utc>>,) = sqlx::query_as(
        "select greatest((select max(updated_at) from health_days), (select max(received_at) from health_workouts), (select max(received_at) from close_signals))",
    )
    .fetch_one(db)
    .await?;
    out.push(format!(
        "Apple Health and Lifesum: {}{}",
        match (key.is_some(), prev_until) {
            (true, Some(t)) => format!("phone key set (the previous key also works until {})", t.format("%Y-%m-%d %H:%M UTC")),
            (true, None) => "phone key set".to_string(),
            (false, _) => "no phone key yet (run: plate phone-key)".to_string(),
        },
        last.0.map(|t| format!(", last delivery {}", t.format("%Y-%m-%d %H:%M UTC"))).unwrap_or_default()
    ));
    for p in [Provider::Strava, Provider::Spotify] {
        let row: Option<(Option<String>, DateTime<Utc>)> = sqlx::query_as("select account, connected_at from provider_tokens where provider = $1").bind(p.key()).fetch_optional(db).await?;
        out.push(match row {
            Some((acc, at)) => format!("{}: connected as {} since {}", p.label(), acc.unwrap_or_else(|| "?".into()), at.format("%Y-%m-%d")),
            None => format!("{}: not connected (run: plate authorize {})", p.label(), p.key()),
        });
    }
    for a in list_agents(db).await? {
        out.push(format!(
            "Agent {}: {}{}",
            a.name,
            if a.revoked { "revoked" } else { "active" },
            if a.revoked { String::new() } else { a.last_used_at.map(|t| format!(", last used {}", t.format("%Y-%m-%d %H:%M UTC"))).unwrap_or_else(|| ", not used yet".into()) }
        ));
    }
    Ok(out)
}
