//! Data posted by iPhone Shortcuts automations with the write-only key.
//!
//! - `POST /ingest/workout-ended` when any Apple Watch workout ends. Its `kind` is fixed in
//!   the automation and says nothing, so it is logged and never used: the ping means only
//!   "a workout just ended" and closes the open session at its last set.
//! - `POST /ingest/health` with a day's totals (and, if a phone can send them, workouts).
//!   Called at 21:30 and after every workout, so many times a day for the same date.
//!
//! The Content-Type is never trusted: both read the raw body, JSON or `key=value` lines.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, TimeZone, Utc};
use gym_core::api::{HealthDay, WorkoutNumbers};
use gym_core::session::{Signal, is_strength_kind};
use tracing::info;
use serde::{Deserialize, Serialize};
use sqlx::types::Json as SqlJson;

use crate::auth::Ingest;
use crate::crypto::sha256_hex;
use crate::error::{AppResult, bad};
use crate::ops;
use crate::state::AppState;
use crate::store;

#[derive(Debug, Serialize)]
pub struct IngestReply {
    pub ok: bool,
    pub outcome: String,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct WorkoutIn {
    pub kind: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub energy_kcal: Option<f64>,
    pub avg_hr: Option<f64>,
    pub max_hr: Option<f64>,
    pub distance_km: Option<f64>,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct HealthIn {
    pub day: Option<HealthDay>,
    pub workouts: Vec<WorkoutIn>,
}

/// "1,234.5", "1.234,5", "412 kcal", "128 bpm" → number.
pub fn parse_number(s: &str) -> Option<f64> {
    let s = s.trim();
    let start = s.find(|c: char| c.is_ascii_digit() || c == '-' || c == '.' || c == ',')?;
    let num: String = s[start..].chars().take_while(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | '-' | '\u{a0}' | ' ' | '\'')).collect();
    let num: String = num.chars().filter(|c| !matches!(c, '\u{a0}' | ' ' | '\'')).collect();
    let cleaned = match (num.rfind('.'), num.rfind(',')) {
        (Some(d), Some(c)) if c > d => num.replace('.', "").replace(',', "."),
        (Some(_), Some(_)) => num.replace(',', ""),
        // A lone comma followed by exactly three digits is a thousands separator.
        (None, Some(c)) if num.len() - c - 1 == 3 && !num[..c].is_empty() => num.replace(',', ""),
        (None, Some(_)) => num.replace(',', "."),
        _ => num,
    };
    let v: f64 = cleaned.parse().ok()?;
    v.is_finite().then_some(v)
}

/// ISO 8601 in its many spellings. Times without an offset use `tz` (e.g. "+01:00").
pub fn parse_time(s: &str, tz: Option<FixedOffset>) -> Option<DateTime<Utc>> {
    let s = s.trim();
    if let Ok(t) = DateTime::parse_from_rfc3339(s) {
        return Some(t.with_timezone(&Utc));
    }
    for f in ["%Y-%m-%d %H:%M:%S %z", "%Y-%m-%dT%H:%M:%S%z", "%Y-%m-%d %H:%M %z", "%Y-%m-%dT%H:%M%z", "%d %b %Y at %H:%M:%S %z"] {
        if let Ok(t) = DateTime::parse_from_str(s, f) {
            return Some(t.with_timezone(&Utc));
        }
    }
    let offset = tz.unwrap_or_else(|| FixedOffset::east_opt(0).expect("utc"));
    for f in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M"] {
        if let Ok(n) = NaiveDateTime::parse_from_str(s, f) {
            return offset.from_local_datetime(&n).single().map(|t| t.with_timezone(&Utc));
        }
    }
    None
}

pub fn parse_offset(s: &str) -> Option<FixedOffset> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("z") || s.eq_ignore_ascii_case("utc") {
        return FixedOffset::east_opt(0);
    }
    let (sign, rest) = match s.as_bytes().first()? {
        b'+' => (1, &s[1..]),
        b'-' => (-1, &s[1..]),
        _ => (1, s),
    };
    let (h, m) = match rest.split_once(':') {
        Some((h, m)) => (h.parse::<i32>().ok()?, m.parse::<i32>().ok()?),
        None if rest.len() == 4 => (rest[..2].parse().ok()?, rest[2..].parse().ok()?),
        None => (rest.parse().ok()?, 0),
    };
    FixedOffset::east_opt(sign * (h * 3600 + m * 60))
}

#[derive(Debug, Deserialize)]
struct WorkoutJson {
    kind: Option<String>,
    #[serde(alias = "type", alias = "name")]
    workout_type: Option<String>,
    start: Option<serde_json::Value>,
    end: Option<serde_json::Value>,
    #[serde(alias = "energy", alias = "kcal", alias = "active_energy")]
    energy_kcal: Option<serde_json::Value>,
    #[serde(alias = "average_heart_rate", alias = "avg_heart_rate")]
    avg_hr: Option<serde_json::Value>,
    #[serde(alias = "max_heart_rate")]
    max_hr: Option<serde_json::Value>,
    #[serde(alias = "distance")]
    distance_km: Option<serde_json::Value>,
}

fn num_v(v: &Option<serde_json::Value>) -> Option<f64> {
    match v {
        Some(serde_json::Value::Number(n)) => n.as_f64(),
        Some(serde_json::Value::String(s)) => parse_number(s),
        _ => None,
    }
}

fn str_v(v: &Option<serde_json::Value>) -> Option<String> {
    match v {
        Some(serde_json::Value::String(s)) => Some(s.clone()),
        Some(serde_json::Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}

/// Read the body of `/ingest/health`, JSON or `key=value` lines.
pub fn parse_health(body: &str) -> Result<HealthIn, String> {
    let body = body.trim();
    let mut map: Vec<(String, String)> = Vec::new();
    let mut workouts_json: Vec<WorkoutJson> = Vec::new();
    if body.starts_with('{') {
        let v: serde_json::Value = serde_json::from_str(body).map_err(|e| format!("The JSON did not parse: {e}"))?;
        let obj = v.as_object().ok_or("Send a JSON object")?;
        for (k, val) in obj {
            if k == "workouts" {
                workouts_json = serde_json::from_value(val.clone()).map_err(|e| format!("workouts: {e}"))?;
            } else if let Some(s) = str_v(&Some(val.clone())) {
                map.push((k.to_ascii_lowercase(), s));
            }
        }
    } else {
        for line in body.lines().flat_map(|l| l.split('&')) {
            let Some((k, v)) = line.split_once('=').or_else(|| line.split_once(':')) else { continue };
            map.push((k.trim().to_ascii_lowercase().replace([' ', '-'], "_"), v.trim().to_string()));
        }
        if map.is_empty() {
            return Err("The body had no key=value lines. Send date=YYYY-MM-DD and the totals, one per line".into());
        }
    }
    let get = |names: &[&str]| map.iter().rev().find(|(k, _)| names.contains(&k.as_str())).map(|(_, v)| v.clone());
    let tz = get(&["tz", "timezone", "offset"]).and_then(|s| parse_offset(&s));

    let mut workouts = Vec::new();
    for w in workouts_json {
        let kind = w.kind.or(w.workout_type).unwrap_or_else(|| "Strength Training".into());
        let (Some(start), Some(end)) = (str_v(&w.start).and_then(|s| parse_time(&s, tz)), str_v(&w.end).and_then(|s| parse_time(&s, tz))) else {
            return Err("Every workout needs a start and an end time".into());
        };
        workouts.push(WorkoutIn { kind, start, end, energy_kcal: num_v(&w.energy_kcal), avg_hr: num_v(&w.avg_hr), max_hr: num_v(&w.max_hr), distance_km: num_v(&w.distance_km) });
    }
    // Text form: workout=kind|start|end|kcal|avg_hr|max_hr, one line per workout.
    for (k, v) in map.iter().filter(|(k, _)| k == "workout") {
        let parts: Vec<&str> = v.split('|').map(str::trim).collect();
        if parts.len() < 3 {
            return Err("A workout line is kind|start|end|kcal|avg_hr|max_hr".into());
        }
        let _ = k;
        let (Some(start), Some(end)) = (parse_time(parts[1], tz), parse_time(parts[2], tz)) else {
            return Err(format!("Could not read the times in: {v}"));
        };
        let n = |i: usize| parts.get(i).and_then(|s| parse_number(s));
        workouts.push(WorkoutIn { kind: parts[0].to_string(), start, end, energy_kcal: n(3), avg_hr: n(4), max_hr: n(5), distance_km: n(6) });
    }
    for w in &workouts {
        if w.end < w.start || (w.end - w.start).num_hours() > 12 {
            return Err("A workout's end is before its start, or it lasts more than 12 hours".into());
        }
    }

    let day = match get(&["date", "day"]) {
        Some(d) => {
            let date = NaiveDate::parse_from_str(d.trim().get(..10).unwrap_or(""), "%Y-%m-%d").map_err(|_| format!("Could not read the date {d:?}. Use date=YYYY-MM-DD"))?;
            // An empty value ("protein=") is no reading, not zero: None, and the store keeps
            // what it had. A locked phone sends exactly that after a workout.
            let n = |names: &[&str]| get(names).and_then(|v| parse_number(&v)).filter(|v| *v >= 0.0 && *v < 100_000.0);
            Some(HealthDay {
                date,
                energy_burned_kcal: n(&["energy_burned_kcal", "active_energy", "energy_burned", "burned", "active_kcal"]),
                energy_eaten_kcal: n(&["energy_eaten_kcal", "dietary_energy", "energy_eaten", "eaten", "calories_eaten"]),
                protein_g: n(&["protein_g", "protein"]),
                carbs_g: n(&["carbs_g", "carbs", "carbohydrates"]),
                fat_g: n(&["fat_g", "fat", "total_fat"]),
            })
        }
        None => None,
    };
    if day.is_none() && workouts.is_empty() {
        return Err("Missing the date line. Send date=YYYY-MM-DD with the totals".into());
    }
    Ok(HealthIn { day, workouts })
}

async fn log_signal(st: &AppState, kind: &str, detail: serde_json::Value, outcome: &str) -> AppResult<()> {
    sqlx::query("insert into close_signals (kind, detail, outcome) values ($1, $2, $3)").bind(kind).bind(SqlJson(detail)).bind(outcome).execute(&st.db).await?;
    Ok(())
}

pub async fn health(_: Ingest, State(st): State<AppState>, body: Bytes) -> AppResult<Json<IngestReply>> {
    let text = std::str::from_utf8(&body).map_err(|_| bad("Send text"))?;
    let parsed = parse_health(text).map_err(bad)?;
    let mut tx = st.db.begin().await?;
    if let Some(d) = &parsed.day {
        store::merge_health_day(&mut tx, d).await?;
    }
    let mut outcomes = Vec::new();
    for w in &parsed.workouts {
        let id = sha256_hex(&format!("{}|{}", w.kind.to_ascii_lowercase(), w.start.timestamp()));
        let strength = is_strength_kind(&w.kind);
        let (session_id, outcome) = store::apply_signal(&mut tx, &Signal::WorkoutRecord { start: w.start, end: w.end, strength }).await?;
        if let Some(sid) = session_id {
            let numbers = WorkoutNumbers {
                duration_min: Some(((w.end - w.start).num_seconds() as f64 / 60.0 * 10.0).round() / 10.0),
                energy_kcal: w.energy_kcal,
                avg_heart_rate: w.avg_hr,
                max_heart_rate: w.max_hr,
            };
            store::attach_workout(&mut tx, sid, &numbers).await?;
        }
        sqlx::query(
            "insert into health_workouts (id, kind, start_at, end_at, energy_kcal, avg_hr, max_hr, distance_km, session_id) values ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             on conflict (id) do update set end_at = excluded.end_at, energy_kcal = coalesce(excluded.energy_kcal, health_workouts.energy_kcal),
               avg_hr = coalesce(excluded.avg_hr, health_workouts.avg_hr), max_hr = coalesce(excluded.max_hr, health_workouts.max_hr),
               distance_km = coalesce(excluded.distance_km, health_workouts.distance_km), session_id = coalesce(excluded.session_id, health_workouts.session_id), received_at = now()",
        )
        .bind(&id)
        .bind(&w.kind)
        .bind(w.start)
        .bind(w.end)
        .bind(w.energy_kcal)
        .bind(w.avg_hr)
        .bind(w.max_hr)
        .bind(w.distance_km)
        .bind(session_id)
        .execute(&mut *tx)
        .await?;
        outcomes.push(format!("{}: {outcome}", w.kind));
    }
    tx.commit().await?;
    if outcomes.iter().any(|o| o.ends_with("closed")) {
        ops::flush_queued(&st.db, &st.catalogue, Utc::now().date_naive()).await?;
    }
    let has_numbers = parsed.day.as_ref().is_some_and(|d| {
        [d.energy_burned_kcal, d.energy_eaten_kcal, d.protein_g, d.carbs_g, d.fat_g].iter().any(|v| v.is_some_and(|v| v > 0.0))
    });
    let outcome = match (outcomes.is_empty(), has_numbers) {
        (true, true) => "Saved the day's totals".to_string(),
        // Health could not be read (phone locked) or nothing logged yet: keep what is stored.
        (true, false) => "No numbers in this delivery, so nothing changed".to_string(),
        (false, _) => format!("Saved. {}", outcomes.join("; ")),
    };
    log_signal(&st, "health", serde_json::json!({"workouts": parsed.workouts.len(), "day": parsed.day.as_ref().map(|d| d.date)}), &outcome).await?;
    Ok(Json(IngestReply { ok: true, outcome }))
}

/// The Shortcuts automation on "Apple Watch Workout ends". It fires for every workout
/// (strength, tennis, runs) and always sends the same `kind`, so `kind` is logged as
/// unreliable and nothing depends on it. An optional `at=<ISO time>` is honoured.
pub async fn workout_ended(_: Ingest, State(st): State<AppState>, body: Bytes) -> AppResult<Json<IngestReply>> {
    let text = std::str::from_utf8(&body).unwrap_or_default().trim().to_string();
    let mut kind: Option<String> = None;
    let mut at: Option<DateTime<Utc>> = None;
    let mut tz: Option<FixedOffset> = None;
    if text.starts_with('{') {
        let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| bad(format!("The JSON did not parse: {e}")))?;
        kind = v.get("kind").or_else(|| v.get("type")).and_then(|x| x.as_str()).map(str::to_string);
        tz = v.get("tz").and_then(|x| x.as_str()).and_then(parse_offset);
        at = v.get("at").or_else(|| v.get("end")).and_then(|x| x.as_str()).and_then(|s| parse_time(s, tz));
    } else {
        for line in text.lines().flat_map(|l| l.split('&')) {
            if let Some((k, v)) = line.split_once('=').or_else(|| line.split_once(':')) {
                match k.trim().to_ascii_lowercase().as_str() {
                    "kind" | "type" => kind = Some(v.trim().to_string()),
                    "tz" => tz = parse_offset(v),
                    "at" | "end" => at = parse_time(v, tz),
                    _ => {}
                }
            }
        }
    }
    let received = Utc::now();
    let at = at.filter(|t| (*t - received).num_minutes().abs() < 24 * 60).unwrap_or(received);
    info!(kind = kind.as_deref().unwrap_or(""), "workout ended (kind is fixed in the automation; not used)");
    let mut tx = st.db.begin().await?;
    let (_, outcome) = store::apply_signal(&mut tx, &Signal::WorkoutEnded { at }).await?;
    tx.commit().await?;
    if outcome == "closed" {
        ops::flush_queued(&st.db, &st.catalogue, received.date_naive()).await?;
    }
    let msg = match outcome {
        "closed" => "Session closed and saved".to_string(),
        "ignored" => "No open session to close".to_string(),
        other => other.to_string(),
    };
    log_signal(&st, "workout_ended", serde_json::json!({"kind": kind, "kind_reliable": false, "at": at, "received_at": received}), &msg).await?;
    Ok(Json(IngestReply { ok: true, outcome: msg }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_in_any_locale() {
        assert_eq!(parse_number("1,234.5"), Some(1234.5));
        assert_eq!(parse_number("1.234,5"), Some(1234.5));
        assert_eq!(parse_number("412 kcal"), Some(412.0));
        assert_eq!(parse_number("128,4"), Some(128.4));
        assert_eq!(parse_number("2,600"), Some(2600.0));
        assert_eq!(parse_number("96 g"), Some(96.0));
        assert_eq!(parse_number("kcal"), None);
    }

    #[test]
    fn times_in_shortcuts_formats() {
        let a = parse_time("2026-11-19T18:54:00+01:00", None).unwrap();
        assert_eq!(a.to_rfc3339(), "2026-11-19T17:54:00+00:00");
        assert_eq!(parse_time("2026-11-19 18:54:00 +0100", None), Some(a));
        assert_eq!(parse_time("2026-11-19T18:54:00", parse_offset("+01:00")), Some(a));
        assert_eq!(parse_offset("-0500").unwrap().local_minus_utc(), -5 * 3600);
    }

    #[test]
    fn text_body_with_a_workout() {
        let body = "date=2026-11-19\nactive_energy=610 kcal\ndietary_energy=1,420\nprotein=96\ntz=+01:00\nworkout=Traditional Strength Training|2026-11-19T18:00:00|2026-11-19T18:54:00|412|128|161";
        let h = parse_health(body).unwrap();
        let d = h.day.unwrap();
        assert_eq!(d.energy_burned_kcal, Some(610.0));
        assert_eq!(d.energy_eaten_kcal, Some(1420.0));
        assert_eq!(h.workouts.len(), 1);
        assert_eq!(h.workouts[0].avg_hr, Some(128.0));
        assert_eq!(h.workouts[0].end.to_rfc3339(), "2026-11-19T17:54:00+00:00");
    }

    #[test]
    fn json_body() {
        let body = r#"{"date":"2026-11-19","energy_eaten_kcal":"1420","workouts":[{"type":"Functional Strength Training","start":"2026-11-19T18:00:00Z","end":"2026-11-19T18:54:00Z","kcal":412}]}"#;
        let h = parse_health(body).unwrap();
        assert_eq!(h.day.unwrap().energy_eaten_kcal, Some(1420.0));
        assert_eq!(h.workouts[0].kind, "Functional Strength Training");
        assert_eq!(h.workouts[0].energy_kcal, Some(412.0));
    }

    #[test]
    fn the_phones_exact_body() {
        let h = parse_health("date=2026-11-20\nactive_energy=412.734\ndietary_energy=2,104.5\nprotein=96.2\ncarbs=210\nfat=71.05\n").unwrap();
        let d = h.day.unwrap();
        assert_eq!(d.date.to_string(), "2026-11-20");
        assert_eq!((d.energy_burned_kcal, d.energy_eaten_kcal, d.protein_g, d.carbs_g, d.fat_g), (Some(412.734), Some(2104.5), Some(96.2), Some(210.0), Some(71.05)));
        assert!(h.workouts.is_empty());
    }

    #[test]
    fn empty_values_are_no_reading() {
        let d = parse_health("date=2026-11-20\nactive_energy=\ndietary_energy=\nprotein=0\ncarbs=\nfat=").unwrap().day.unwrap();
        assert_eq!((d.energy_burned_kcal, d.energy_eaten_kcal, d.carbs_g, d.fat_g), (None, None, None, None));
        assert_eq!(d.protein_g, Some(0.0), "0 parses as 0; the store refuses to let it replace a number");
    }

    #[test]
    fn crlf_and_trailing_blank_lines() {
        let d = parse_health("date=2026-11-20\r\nprotein=96\r\n\r\n").unwrap().day.unwrap();
        assert_eq!(d.protein_g, Some(96.0));
    }

    #[test]
    fn refuses_nonsense() {
        assert!(parse_health("").unwrap_err().contains("no key=value"));
        assert!(parse_health("active_energy=412\nprotein=96").unwrap_err().contains("date"));
        assert!(parse_health("hello").is_err());
        assert!(parse_health("date=yesterday").is_err());
        assert!(parse_health("workout=Run|2026-11-19T18:00:00Z|2026-11-19T17:00:00Z").is_err());
    }
}
