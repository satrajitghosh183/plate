//! Talking to the backend. Same origin: on Vercel, `/api` is forwarded to Render.
//!
//! Render's free plan sleeps after 15 quiet minutes and takes about a minute to wake.
//! While it wakes, the forwarder can answer with an error or a page that is not
//! JSON, so those count as "still waking" and the caller tries again.

use serde::Serialize;
use serde::de::DeserializeOwned;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Request, RequestCredentials, RequestInit, Response};

#[derive(Debug, Clone, PartialEq)]
pub enum NetError {
    /// No network, or the backend is waking up. Try again later.
    Unreachable,
    /// Signed out or the session expired.
    Unauthorized,
    /// The backend said no, with a reason worth showing.
    Refused(u16, String),
}

impl NetError {
    pub fn message(&self) -> String {
        match self {
            NetError::Unreachable => "Can't reach the server right now".into(),
            NetError::Unauthorized => "Sign in again".into(),
            NetError::Refused(_, m) => m.clone(),
        }
    }
}

pub fn tz() -> i32 {
    crate::bridge::timezone_offset()
}

fn with_tz(path: &str) -> String {
    let sep = if path.contains('?') { '&' } else { '?' };
    format!("{path}{sep}tz={}", tz())
}

pub fn online() -> bool {
    web_sys::window().map(|w| w.navigator().on_line()).unwrap_or(true)
}

async fn send(method: &str, path: &str, body: Option<String>) -> Result<(u16, String), NetError> {
    if !online() {
        return Err(NetError::Unreachable);
    }
    let init = RequestInit::new();
    init.set_method(method);
    init.set_credentials(RequestCredentials::SameOrigin);
    if let Some(b) = &body {
        init.set_body(&JsValue::from_str(b));
    }
    let req = Request::new_with_str_and_init(&with_tz(path), &init).map_err(|_| NetError::Unreachable)?;
    let h = req.headers();
    let _ = h.set("accept", "application/json");
    if body.is_some() {
        let _ = h.set("content-type", "application/json");
    }
    let window = web_sys::window().ok_or(NetError::Unreachable)?;
    let resp: Response = JsFuture::from(window.fetch_with_request(&req)).await.map_err(|_| NetError::Unreachable)?.dyn_into().map_err(|_| NetError::Unreachable)?;
    let status = resp.status();
    let is_json = resp.headers().get("content-type").ok().flatten().is_some_and(|c| c.contains("json"));
    // Vercel's own errors (a deployment without the rewrite, a timeout while Render wakes)
    // carry this header and a JSON body of a different shape. They are not Plate saying no.
    let platform_error = resp.headers().get("x-vercel-error").ok().flatten().is_some();
    let text = match resp.text() {
        Ok(p) => JsFuture::from(p).await.ok().and_then(|v| v.as_string()).unwrap_or_default(),
        Err(_) => String::new(),
    };
    match status {
        _ if platform_error => Err(NetError::Unreachable),
        401 => Err(NetError::Unauthorized),
        502..=504 => Err(NetError::Unreachable),
        200..=299 if status != 204 && !is_json && !text.is_empty() => Err(NetError::Unreachable),
        200..=299 => Ok((status, text)),
        _ if !is_json => Err(NetError::Unreachable),
        _ => {
            let msg = serde_json::from_str::<gym_core::api::ApiError>(&text).map(|e| e.error).unwrap_or_else(|_| format!("The server said {status}"));
            Err(NetError::Refused(status, msg))
        }
    }
}

pub async fn get<T: DeserializeOwned>(path: &str) -> Result<T, NetError> {
    let (_, text) = send("GET", path, None).await?;
    serde_json::from_str(&text).map_err(|_| NetError::Unreachable)
}

pub async fn post<B: Serialize, T: DeserializeOwned>(path: &str, body: &B) -> Result<T, NetError> {
    write("POST", path, body).await
}

pub async fn write<B: Serialize, T: DeserializeOwned>(method: &str, path: &str, body: &B) -> Result<T, NetError> {
    let (_, text) = send(method, path, Some(serde_json::to_string(body).unwrap_or_default())).await?;
    serde_json::from_str(&text).map_err(|_| NetError::Refused(500, "The server sent something unexpected".into()))
}

/// For calls whose reply has no body worth reading.
pub async fn call<B: Serialize>(method: &str, path: &str, body: Option<&B>) -> Result<(), NetError> {
    send(method, path, body.map(|b| serde_json::to_string(b).unwrap_or_default())).await.map(|_| ())
}
