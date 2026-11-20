//! Who is calling.
//!
//! - You: a passkey, then an HttpOnly session cookie. The first passkey needs the
//!   setup code; after that, only a signed-in session can add another.
//! - Agents: one key each, stored hashed, revocable, rate limited.
//! - The phone's Shortcuts: one write-only key that can post health data and the
//!   workout-ended ping and read nothing.

use axum::Json;
use axum::extract::{FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use chrono::{Duration, Utc};
use gym_core::api::AuthStatus;
use serde::Deserialize;
use sqlx::types::Json as SqlJson;
use uuid::Uuid;
use webauthn_rs::prelude::{Passkey, PasskeyAuthentication, PasskeyRegistration, PublicKeyCredential, RegisterPublicKeyCredential};

use crate::crypto::{constant_time_eq, new_secret, sha256_hex};
use crate::error::{AppError, AppResult, bad};
use crate::state::AppState;

pub const COOKIE: &str = "gym_session";
const SESSION_DAYS: i64 = 60;
const OWNER_ID: Uuid = Uuid::from_u128(0x6779_6d00_0000_4000_8000_0000_0000_0001);

/// Proof that the request comes from your signed-in browser.
#[derive(Debug, Clone, Copy)]
pub struct Owner;

impl FromRequestParts<AppState> for Owner {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> Result<Self, Self::Rejection> {
        let jar = CookieJar::from_headers(&parts.headers);
        let token = jar.get(COOKIE).map(|c| c.value().to_string()).ok_or(AppError::Unauthorized)?;
        if signed_in(st, &token).await? { Ok(Owner) } else { Err(AppError::Unauthorized) }
    }
}

async fn signed_in(st: &AppState, token: &str) -> AppResult<bool> {
    let row: Option<(String,)> = sqlx::query_as("update auth_sessions set last_seen_at = now() where token_hash = $1 and expires_at > now() returning token_hash")
    .bind(sha256_hex(token))
    .fetch_optional(&st.db)
    .await?;
    Ok(row.is_some())
}

/// Writes from the browser must come from the app's own origin. Requests without
/// an Origin header (curl, agents, Shortcuts) do not reach cookie routes signed in.
pub async fn same_origin(State(st): State<AppState>, req: axum::extract::Request, next: Next) -> Response {
    let safe = matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    if !safe
        && let Some(origin) = req.headers().get("origin").and_then(|o| o.to_str().ok())
        && origin != st.cfg.app_origin()
        && origin != st.cfg.public_url.origin().ascii_serialization()
    {
        return AppError::Forbidden("This request came from another site".into()).into_response();
    }
    next.run(req).await
}

async fn passkey_count(st: &AppState) -> AppResult<i64> {
    let n: (i64,) = sqlx::query_as("select count(*) from passkeys").fetch_one(&st.db).await?;
    Ok(n.0)
}

pub async fn status(State(st): State<AppState>, jar: CookieJar) -> AppResult<Json<AuthStatus>> {
    let signed = match jar.get(COOKIE) {
        Some(c) => signed_in(&st, c.value()).await?,
        None => false,
    };
    Ok(Json(AuthStatus { signed_in: signed, needs_setup: passkey_count(&st).await? == 0 }))
}

#[derive(Deserialize)]
pub struct RegisterStart {
    #[serde(default)]
    pub setup_code: Option<String>,
}

#[derive(serde::Serialize)]
pub struct Ceremony<T> {
    pub ceremony: Uuid,
    pub options: T,
}

async fn save_ceremony<T: serde::Serialize>(st: &AppState, kind: &str, state: &T) -> AppResult<Uuid> {
    sqlx::query("delete from ceremonies where expires_at < now()").execute(&st.db).await?;
    let id = Uuid::new_v4();
    sqlx::query("insert into ceremonies (id, kind, state, expires_at) values ($1, $2, $3, $4)")
        .bind(id)
        .bind(kind)
        .bind(SqlJson(serde_json::to_value(state)?))
        .bind(Utc::now() + Duration::minutes(5))
        .execute(&st.db)
        .await?;
    Ok(id)
}

async fn take_ceremony<T: serde::de::DeserializeOwned>(st: &AppState, id: Uuid, kind: &str) -> AppResult<T> {
    let row: Option<(SqlJson<serde_json::Value>,)> =
        sqlx::query_as("delete from ceremonies where id = $1 and kind = $2 and expires_at > now() returning state").bind(id).bind(kind).fetch_optional(&st.db).await?;
    let (SqlJson(v),) = row.ok_or_else(|| bad("That sign-in attempt expired. Start again"))?;
    Ok(serde_json::from_value(v)?)
}

/// Start adding a passkey. The first one needs the setup code; later ones need you signed in.
pub async fn register_start(State(st): State<AppState>, jar: CookieJar, Json(body): Json<RegisterStart>) -> AppResult<impl IntoResponse> {
    if passkey_count(&st).await? == 0 {
        let Some(expected) = st.cfg.setup_code.as_deref() else {
            return Err(AppError::Forbidden("Setup is closed: SETUP_CODE is not set on the server".into()));
        };
        let given = body.setup_code.unwrap_or_default();
        if !constant_time_eq(given.trim(), expected) {
            return Err(AppError::Forbidden("That setup code is wrong".into()));
        }
    } else {
        let token = jar.get(COOKIE).map(|c| c.value().to_string()).ok_or(AppError::Forbidden("A passkey already exists. Sign in with it to add another".into()))?;
        if !signed_in(&st, &token).await? {
            return Err(AppError::Forbidden("A passkey already exists. Sign in with it to add another".into()));
        }
    }
    // Only now, with permission settled, read the stored credentials.
    let exclude = load_passkeys(&st).await?.iter().map(|(_, p)| p.cred_id().clone()).collect::<Vec<_>>();
    let (options, state) = st.webauthn.start_passkey_registration(OWNER_ID, "me", "Me", Some(exclude)).map_err(|e| bad(format!("Could not start: {e}")))?;
    let ceremony = save_ceremony(&st, "register", &state).await?;
    Ok(Json(Ceremony { ceremony, options }))
}

#[derive(Deserialize)]
pub struct RegisterFinish {
    pub ceremony: Uuid,
    pub credential: RegisterPublicKeyCredential,
    #[serde(default)]
    pub name: Option<String>,
}

pub async fn register_finish(State(st): State<AppState>, jar: CookieJar, Json(body): Json<RegisterFinish>) -> AppResult<impl IntoResponse> {
    let state: PasskeyRegistration = take_ceremony(&st, body.ceremony, "register").await?;
    let passkey = st.webauthn.finish_passkey_registration(&body.credential, &state).map_err(|e| bad(format!("The passkey was not accepted: {e}")))?;
    let name = body.name.map(|n| n.trim().chars().take(40).collect::<String>()).filter(|n| !n.is_empty()).unwrap_or_else(|| "iPhone".into());
    // Re-check under the insert: two first-registrations racing must not both win.
    let mut tx = st.db.begin().await?;
    sqlx::query("lock table passkeys in exclusive mode").execute(&mut *tx).await?;
    let n: (i64,) = sqlx::query_as("select count(*) from passkeys").fetch_one(&mut *tx).await?;
    let has_cookie = match jar.get(COOKIE) {
        Some(c) => signed_in(&st, c.value()).await?,
        None => false,
    };
    if n.0 > 0 && !has_cookie {
        return Err(AppError::Forbidden("A passkey already exists. Sign in with it to add another".into()));
    }
    sqlx::query("insert into passkeys (id, name, cred) values ($1, $2, $3)")
        .bind(Uuid::new_v4())
        .bind(&name)
        .bind(SqlJson(serde_json::to_value(&passkey)?))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let jar = start_session(&st, jar).await?;
    Ok((jar, Json(AuthStatus { signed_in: true, needs_setup: false })))
}

async fn load_passkeys(st: &AppState) -> AppResult<Vec<(Uuid, Passkey)>> {
    let rows: Vec<(Uuid, SqlJson<serde_json::Value>)> = sqlx::query_as("select id, cred from passkeys").fetch_all(&st.db).await?;
    rows.into_iter().map(|(id, SqlJson(v))| Ok((id, serde_json::from_value(v)?))).collect()
}

pub async fn login_start(State(st): State<AppState>) -> AppResult<impl IntoResponse> {
    let keys: Vec<Passkey> = load_passkeys(&st).await?.into_iter().map(|(_, p)| p).collect();
    if keys.is_empty() {
        return Err(AppError::Conflict("No passkey yet. Set one up first".into()));
    }
    let (options, state) = st.webauthn.start_passkey_authentication(&keys).map_err(|e| bad(format!("Could not start: {e}")))?;
    let ceremony = save_ceremony(&st, "login", &state).await?;
    Ok(Json(Ceremony { ceremony, options }))
}

#[derive(Deserialize)]
pub struct LoginFinish {
    pub ceremony: Uuid,
    pub credential: PublicKeyCredential,
}

pub async fn login_finish(State(st): State<AppState>, jar: CookieJar, Json(body): Json<LoginFinish>) -> AppResult<impl IntoResponse> {
    let state: PasskeyAuthentication = take_ceremony(&st, body.ceremony, "login").await?;
    let result = st.webauthn.finish_passkey_authentication(&body.credential, &state).map_err(|_| AppError::Forbidden("That passkey did not check out".into()))?;
    // Keep the signature counter and backup flags current.
    for (id, mut pk) in load_passkeys(&st).await? {
        if pk.cred_id() == result.cred_id() {
            if pk.update_credential(&result).unwrap_or(false) {
                sqlx::query("update passkeys set cred = $2 where id = $1").bind(id).bind(SqlJson(serde_json::to_value(&pk)?)).execute(&st.db).await?;
            }
            sqlx::query("update passkeys set last_used_at = now() where id = $1").bind(id).execute(&st.db).await?;
        }
    }
    let jar = start_session(&st, jar).await?;
    Ok((jar, Json(AuthStatus { signed_in: true, needs_setup: false })))
}

async fn start_session(st: &AppState, jar: CookieJar) -> AppResult<CookieJar> {
    let token = new_secret("s");
    sqlx::query("delete from auth_sessions where expires_at < now()").execute(&st.db).await?;
    sqlx::query("insert into auth_sessions (token_hash, expires_at) values ($1, $2)")
        .bind(sha256_hex(&token))
        .bind(Utc::now() + Duration::days(SESSION_DAYS))
        .execute(&st.db)
        .await?;
    let cookie = Cookie::build((COOKIE, token))
        .http_only(true)
        .secure(st.cfg.cookie_secure)
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(time_dur(SESSION_DAYS))
        .build();
    Ok(jar.add(cookie))
}

fn time_dur(days: i64) -> time::Duration {
    time::Duration::days(days)
}

pub async fn logout(State(st): State<AppState>, jar: CookieJar) -> AppResult<impl IntoResponse> {
    if let Some(c) = jar.get(COOKIE) {
        sqlx::query("delete from auth_sessions where token_hash = $1").bind(sha256_hex(c.value())).execute(&st.db).await?;
    }
    let jar = jar.remove(Cookie::build(COOKIE).path("/").build());
    Ok((jar, StatusCode::NO_CONTENT))
}

// ---------- agents ----------

#[derive(Debug, Clone)]
pub struct AgentCaller {
    pub id: Uuid,
    pub name: String,
    pub ask_first: bool,
}

fn bearer(headers: &HeaderMap) -> Option<String> {
    let v = headers.get("authorization")?.to_str().ok()?;
    let (scheme, token) = v.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then(|| token.trim().to_string())
}

pub async fn agent_by_key(st: &AppState, key: &str) -> AppResult<AgentCaller> {
    if !key.starts_with("agent_") {
        return Err(AppError::Unauthorized);
    }
    let row: Option<(Uuid, String, bool)> = sqlx::query_as(
        "update agents set last_used_at = now() where key_hash = $1 and revoked_at is null returning id, name, ask_first",
    )
    .bind(sha256_hex(key))
    .fetch_optional(&st.db)
    .await?;
    let (id, name, ask_first) = row.ok_or(AppError::Unauthorized)?;
    if !st.limiter.allow(id) {
        return Err(AppError::TooMany);
    }
    Ok(AgentCaller { id, name, ask_first })
}

/// The agent key from `Authorization: Bearer …`, or from the link's path.
pub async fn agent_from(st: &AppState, headers: &HeaderMap, path_key: Option<&str>) -> AppResult<AgentCaller> {
    let key = path_key.map(str::to_string).or_else(|| bearer(headers)).ok_or(AppError::Unauthorized)?;
    agent_by_key(st, &key).await
}

// ---------- the phone's write-only key ----------

pub struct Ingest;

impl FromRequestParts<AppState> for Ingest {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> Result<Self, Self::Rejection> {
        let key = parts.headers.get("x-ingest-key").and_then(|v| v.to_str().ok()).map(str::to_string).or_else(|| bearer(&parts.headers)).ok_or(AppError::Unauthorized)?;
        let mut c = st.db.acquire().await?;
        let stored: Option<String> = crate::store::get_setting(&mut c, "ingest_key_hash").await?;
        match stored {
            Some(h) if constant_time_eq(&h, &sha256_hex(key.trim())) => Ok(Ingest),
            _ => Err(AppError::Unauthorized),
        }
    }
}

/// Slow down guessing: sign-in and the phone's intake allow 30 requests a minute per
/// address. Behind Render's proxy the caller's address is the first X-Forwarded-For entry.
pub async fn throttle(State(st): State<AppState>, req: axum::extract::Request, next: Next) -> Response {
    // Inside the nested /api router the path has lost its prefix; use the original.
    let path = req.extensions().get::<axum::extract::OriginalUri>().map(|u| u.0.path().to_string()).unwrap_or_else(|| req.uri().path().to_string());
    let path = path.as_str();
    let guarded = path.starts_with("/api/auth/") || path.starts_with("/ingest/");
    if guarded {
        let ip = req
            .headers()
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .map(|v| v.trim().to_string())
            .or_else(|| req.extensions().get::<axum::extract::ConnectInfo<std::net::SocketAddr>>().map(|c| c.0.ip().to_string()))
            .unwrap_or_else(|| "unknown".into());
        let bucket = if path.starts_with("/ingest/") { "ingest" } else { "auth" };
        if !st.throttle.allow(format!("{bucket}:{ip}")) {
            return AppError::TooMany.into_response();
        }
    }
    next.run(req).await
}
