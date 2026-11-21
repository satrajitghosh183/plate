use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gym_core::catalogue::Catalogue;
use sqlx::PgPool;
use webauthn_rs::Webauthn;

use crate::config::Config;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub cfg: Arc<Config>,
    pub webauthn: Arc<Webauthn>,
    pub catalogue: Arc<Catalogue>,
    pub http: reqwest::Client,
    /// Agents: 120 calls a minute each.
    pub limiter: Arc<RateLimiter>,
    /// Sign-in and the phone's intake: 30 calls a minute per address.
    pub throttle: Arc<RateLimiter>,
    /// The same paths in total, whatever address a caller claims: forged
    /// X-Forwarded-For values cannot multiply the guesses.
    pub throttle_all: Arc<RateLimiter>,
    /// The agent link per address, before any key is checked: 240 a minute.
    pub throttle_mcp: Arc<RateLimiter>,
}

/// A fixed-window limit per caller. Memory stays bounded: windows older than two
/// minutes are dropped, and past `MAX_KEYS` callers every new one shares a single window.
pub struct RateLimiter {
    per_minute: u32,
    windows: Mutex<HashMap<String, (Instant, u32)>>,
}

impl RateLimiter {
    pub fn new(per_minute: u32) -> Self {
        RateLimiter { per_minute, windows: Mutex::new(HashMap::new()) }
    }

    pub fn allow(&self, who: impl ToString) -> bool {
        const MAX_KEYS: usize = 4_096;
        let mut w = self.windows.lock().expect("limiter lock");
        let now = Instant::now();
        // Forget callers whose window ended, so the map stays small.
        if w.len() >= MAX_KEYS / 2 {
            w.retain(|_, (t, _)| now.duration_since(*t) < Duration::from_secs(120));
        }
        let mut key = who.to_string();
        if key.len() > 64 {
            key.truncate(64);
        }
        if !w.contains_key(&key) && w.len() >= MAX_KEYS {
            key = "overflow".into();
        }
        let entry = w.entry(key).or_insert((now, 0));
        if now.duration_since(entry.0) >= Duration::from_secs(60) {
            *entry = (now, 0);
        }
        entry.1 += 1;
        entry.1 <= self.per_minute
    }
}

pub fn load_catalogue() -> Catalogue {
    Catalogue::from_json(include_str!("../../../data/catalogue/exercises.min.json")).expect("bundled catalogue parses")
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn limiter_stops_after_the_limit() {
        let l = RateLimiter::new(3);
        let id = Uuid::new_v4();
        assert!(l.allow(id) && l.allow(id) && l.allow(id));
        assert!(!l.allow(id));
        assert!(l.allow(Uuid::new_v4()), "limits are per agent");
    }

    #[test]
    fn limiter_memory_is_bounded() {
        let l = RateLimiter::new(1);
        for i in 0..20_000 {
            l.allow(format!("caller-{i}-{}", "x".repeat(500)));
        }
        let w = l.windows.lock().unwrap();
        assert!(w.len() <= 4_097, "{} keys kept", w.len());
        assert!(w.keys().all(|k| k.len() <= 64));
    }
}
