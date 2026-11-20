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
}

/// A fixed-window limit per caller. One person, a few agents: memory is fine.
pub struct RateLimiter {
    per_minute: u32,
    windows: Mutex<HashMap<String, (Instant, u32)>>,
}

impl RateLimiter {
    pub fn new(per_minute: u32) -> Self {
        RateLimiter { per_minute, windows: Mutex::new(HashMap::new()) }
    }

    pub fn allow(&self, who: impl ToString) -> bool {
        let mut w = self.windows.lock().expect("limiter lock");
        let now = Instant::now();
        // Forget callers whose window ended long ago, so the map stays small.
        if w.len() > 10_000 {
            w.retain(|_, (t, _)| now.duration_since(*t) < Duration::from_secs(120));
        }
        let entry = w.entry(who.to_string()).or_insert((now, 0));
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
}
