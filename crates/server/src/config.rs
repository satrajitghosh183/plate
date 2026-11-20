//! Settings read from the environment once at start-up.

use anyhow::{Context, bail};
use url::Url;

#[derive(Debug, Clone)]
pub struct ProviderApp {
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub port: u16,
    /// Where the phone app is served from, e.g. https://gym-app.vercel.app.
    /// Passkeys are bound to this host and OAuth callbacks come back through it.
    pub app_url: Url,
    /// This backend's own public address, used in agent links and the Strava webhook.
    pub public_url: Url,
    /// The passkey relying-party id: the app's host name.
    pub rp_id: String,
    /// Required to register the first passkey. Unset means nobody can register.
    pub setup_code: Option<String>,
    /// 32-byte key (64 hex characters) that encrypts provider tokens.
    pub token_key: Option<[u8; 32]>,
    pub cookie_secure: bool,
    /// Serve the built phone app from this folder too (the "drop Vercel" fallback).
    pub static_dir: Option<String>,
    pub strava: Option<ProviderApp>,
    pub strava_verify_token: Option<String>,
    pub spotify: Option<ProviderApp>,
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// 32 bytes as 64 hex characters, or base64 (what Render's "generate value" makes).
fn parse_key(k: &str) -> anyhow::Result<[u8; 32]> {
    use base64::Engine;
    let bytes = if k.len() == 64 && k.chars().all(|c| c.is_ascii_hexdigit()) {
        hex::decode(k)?
    } else {
        base64::engine::general_purpose::STANDARD.decode(k).or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(k)).context("TOKEN_KEY must be hex or base64")?
    };
    let Ok(key) = <[u8; 32]>::try_from(bytes.as_slice()) else { bail!("TOKEN_KEY must hold exactly 32 bytes") };
    Ok(key)
}

fn provider(id: &str, secret: &str) -> Option<ProviderApp> {
    Some(ProviderApp { client_id: var(id)?, client_secret: var(secret)? })
}

impl Config {
    pub fn from_env() -> anyhow::Result<Config> {
        let port = var("PORT").map(|p| p.parse()).transpose().context("PORT must be a number")?.unwrap_or(8080);
        let app_url = Url::parse(&var("APP_URL").unwrap_or_else(|| format!("http://localhost:{port}"))).context("APP_URL is not a URL")?;
        let public_url = Url::parse(&var("PUBLIC_URL").unwrap_or_else(|| app_url.to_string())).context("PUBLIC_URL is not a URL")?;
        let rp_id = var("RP_ID").or_else(|| app_url.host_str().map(str::to_string)).context("RP_ID or APP_URL with a host is required")?;
        let token_key = match var("TOKEN_KEY") {
            Some(k) => Some(parse_key(&k)?),
            None => None,
        };
        Ok(Config {
            database_url: var("DATABASE_URL").context("DATABASE_URL is required")?,
            port,
            cookie_secure: app_url.scheme() == "https",
            app_url,
            public_url,
            rp_id,
            setup_code: var("SETUP_CODE"),
            token_key,
            static_dir: var("STATIC_DIR"),
            strava: provider("STRAVA_CLIENT_ID", "STRAVA_CLIENT_SECRET"),
            strava_verify_token: var("STRAVA_VERIFY_TOKEN"),
            spotify: provider("SPOTIFY_CLIENT_ID", "SPOTIFY_CLIENT_SECRET"),
        })
    }

    /// The origin string browsers send in the Origin header.
    pub fn app_origin(&self) -> String {
        self.app_url.origin().ascii_serialization()
    }

    pub fn app_link(&self, path: &str) -> String {
        format!("{}{}", self.app_url.as_str().trim_end_matches('/'), path)
    }

    pub fn public_link(&self, path: &str) -> String {
        format!("{}{}", self.public_url.as_str().trim_end_matches('/'), path)
    }
}

#[cfg(test)]
mod tests {
    use super::parse_key;

    #[test]
    fn token_key_accepts_hex_and_base64() {
        use base64::Engine;
        use rand::RngCore;
        let mut key = [0u8; 32];
        rand::rng().fill_bytes(&mut key);
        assert_eq!(parse_key(&hex::encode(key)).unwrap(), key);
        assert_eq!(parse_key(&base64::engine::general_purpose::STANDARD.encode(key)).unwrap(), key);
        assert!(parse_key("short").is_err());
    }
}
