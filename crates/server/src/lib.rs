//! The gym app's backend: one Axum service.

pub mod admin;
pub mod api;
pub mod auth;
pub mod clips;
pub mod config;
pub mod crypto;
pub mod error;
pub mod ingest;
pub mod mcp;
pub mod ops;
pub mod providers;
pub mod state;
pub mod store;

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::http::{HeaderName, HeaderValue, header};
use axum::routing::{get, patch, post, put};
use sqlx::postgres::PgPoolOptions;
use tower_http::compression::CompressionLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;
use webauthn_rs::prelude::{Url, WebauthnBuilder};

use crate::config::Config;
use crate::state::{AppState, RateLimiter, load_catalogue};

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

pub async fn build_state(cfg: Config) -> anyhow::Result<AppState> {
    let db = PgPoolOptions::new()
        .max_connections(5)
        // Neon pauses when idle; give it time to wake.
        .acquire_timeout(Duration::from_secs(20))
        .connect(&cfg.database_url)
        .await?;
    MIGRATOR.run(&db).await?;
    let origin = Url::parse(&cfg.app_origin())?;
    let mut builder = WebauthnBuilder::new(&cfg.rp_id, &origin)?.rp_name("Plate");
    if cfg.public_url.origin() != cfg.app_url.origin() {
        builder = builder.append_allowed_origin(&Url::parse(&cfg.public_url.origin().ascii_serialization())?);
    }
    let webauthn = builder.build()?;
    let http = reqwest::Client::builder().timeout(Duration::from_secs(15)).user_agent(concat!("plate/", env!("CARGO_PKG_VERSION"))).build()?;
    Ok(AppState { db, cfg: Arc::new(cfg), webauthn: Arc::new(webauthn), catalogue: Arc::new(load_catalogue()), http, limiter: Arc::new(RateLimiter::new(120)), throttle: Arc::new(RateLimiter::new(30)), throttle_all: Arc::new(RateLimiter::new(300)), throttle_mcp: Arc::new(RateLimiter::new(240)) })
}

pub fn router(st: AppState) -> Router {
    let owner_api = Router::new()
        .route("/bootstrap", get(api::bootstrap))
        .route("/settings", put(api::put_settings))
        .route("/lifts", post(api::add_lift))
        .route("/lifts/{id}", patch(api::patch_lift))
        .route("/lifts/{id}/resort", post(api::resort_lift))
        .route("/lifts/{id}/clips", post(api::add_clip))
        .route("/lifts/{id}/progress", get(api::lift_progress))
        .route("/clips/{id}", axum::routing::delete(api::delete_clip))
        .route("/plan/switch", post(api::switch_split))
        .route("/plan/undo", post(api::undo_plan))
        .route("/plan/meta", put(api::plan_meta))
        .route("/plan/start", post(api::plan_start))
        .route("/plan/starter", post(api::load_starter))
        .route("/sync", post(api::sync))
        .route("/sessions", get(api::sessions))
        .route("/sessions/{id}", get(api::session))
        .route("/health", get(api::health))
        .route("/agents", get(api::agents))
        .route("/agents/{id}", patch(api::patch_agent))
        .route("/activity", get(api::activity))
        .route("/activity/{id}/{action}", post(api::activity_action))
        .route("/export", get(api::export))
        .route("/connections", get(providers::connections_route))
        .route("/music/now", get(providers::now_playing))
        .route("/music/playlists", get(providers::playlists))
        .route("/music/command", post(providers::command))
        .route("/music/devices", get(providers::devices))
        .route("/auth/status", get(auth::status))
        .route("/auth/register/start", post(auth::register_start))
        .route("/auth/register/finish", post(auth::register_finish))
        .route("/auth/login/start", post(auth::login_start))
        .route("/auth/login/finish", post(auth::login_finish))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/password", post(auth::password_login))
        .route("/auth/google/start", get(auth::google_start))
        .route("/auth/google/callback", get(auth::google_callback))
        .layer(axum::middleware::from_fn_with_state(st.clone(), auth::same_origin))
        .layer(axum::middleware::from_fn_with_state(st.clone(), auth::throttle))
        .layer(SetResponseHeaderLayer::overriding(header::CACHE_CONTROL, HeaderValue::from_static("no-store")));

    let mut app = Router::new()
        .route("/healthz", get(|| async { axum::Json(serde_json::json!({ "ok": true })) }))
        .nest("/api", owner_api)
        .merge(
            Router::new()
                .route("/ingest/health", post(ingest::health))
                .route("/ingest/workout-ended", post(ingest::workout_ended))
                .layer(axum::middleware::from_fn_with_state(st.clone(), auth::throttle)),
        )
        .merge(
            Router::new()
                .route("/mcp", post(mcp::post).get(mcp::not_allowed).delete(mcp::not_allowed))
                .route("/mcp/{key}", post(mcp::post_with_key).get(mcp::not_allowed).delete(mcp::not_allowed))
                .layer(axum::middleware::from_fn_with_state(st.clone(), auth::throttle)),
        );

    if let Some(dir) = st.cfg.static_dir.clone() {
        let index = format!("{dir}/index.html");
        app = app.fallback_service(ServeDir::new(&dir).fallback(ServeFile::new(index)));
    }

    app.layer(DefaultBodyLimit::max(1024 * 1024))
        .layer(TimeoutLayer::with_status_code(axum::http::StatusCode::REQUEST_TIMEOUT, Duration::from_secs(30)))
        .layer(CompressionLayer::new())
        .layer(SetResponseHeaderLayer::if_not_present(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")))
        .layer(SetResponseHeaderLayer::if_not_present(header::REFERRER_POLICY, HeaderValue::from_static("strict-origin-when-cross-origin")))
        .layer(SetResponseHeaderLayer::if_not_present(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY")))
        .layer(SetResponseHeaderLayer::if_not_present(HeaderName::from_static("permissions-policy"), HeaderValue::from_static("camera=(), microphone=(), geolocation=(), payment=()")))
        .layer(SetResponseHeaderLayer::if_not_present(header::STRICT_TRANSPORT_SECURITY, HeaderValue::from_static("max-age=63072000; includeSubDomains")))
        .layer(TraceLayer::new_for_http().make_span_with(|req: &axum::http::Request<axum::body::Body>| {
            tracing::debug_span!("request", method = %req.method(), path = %log_path(req.uri().path()))
        }))
        .with_state(st)
}

/// The path as it may appear in logs: an agent link carries its key, so that part is hidden.
pub fn log_path(path: &str) -> String {
    match path.strip_prefix("/mcp/") {
        Some(rest) if !rest.is_empty() => "/mcp/[key]".to_string(),
        _ => path.to_string(),
    }
}

/// Close idle sessions and apply waiting agent changes every few minutes.
/// On Render's free plan this only runs while the service is awake; the same check
/// also runs at the start of every app request, so nothing is missed.
pub fn spawn_housekeeping(st: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(300));
        loop {
            tick.tick().await;
            let now = chrono::Utc::now();
            let closed = match st.db.acquire().await {
                Ok(mut c) => store::close_if_idle(&mut c, now).await.ok().flatten().is_some(),
                Err(_) => false,
            };
            if closed {
                let _ = ops::flush_queued(&st.db, &st.catalogue, now.date_naive()).await;
            }
            // Kept out of the phone's request path, so a cold-started call does no extra work.
            let _ = sqlx::query("delete from close_signals where received_at < now() - interval '60 days'").execute(&st.db).await;
        }
    });
}
