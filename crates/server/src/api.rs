//! Routes for the phone app. Every handler here needs `Owner` (your session cookie).

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use chrono::{Duration, NaiveDate, Utc};
use gym_core::api::{
    Activity, Agent, Bootstrap, HealthDay, LiftPatch, LiftProgress, NewClip, NewLift, Plan, Prescription, ProgressPoint, Session, Settings, SwitchSplit,
    SyncResult, SyncSets,
};
use gym_core::progress::best_e1rm;
use gym_core::session::{Decision, SessionWindow, Signal, decide};
use gym_core::{Unit, Weight};
use serde::Deserialize;
use sqlx::types::Json as SqlJson;
use uuid::Uuid;

use crate::auth::Owner;
use crate::error::{AppError, AppResult, bad, not_found};
use crate::ops::{self, Actor, Op, PlanLift, WritePlan};
use crate::providers;
use crate::state::AppState;
use crate::store::{self, local_date};

#[derive(Deserialize)]
pub struct Tz {
    /// JavaScript's getTimezoneOffset(): minutes to add to local time to get UTC.
    #[serde(default)]
    pub tz: i32,
}

impl Tz {
    pub fn today(&self) -> NaiveDate {
        local_date(Utc::now(), self.tz.clamp(-900, 900))
    }
}

pub async fn bootstrap(_: Owner, State(st): State<AppState>, Query(tz): Query<Tz>) -> AppResult<Json<Bootstrap>> {
    let now = Utc::now();
    let today = tz.today();
    let mut c = st.db.acquire().await?;
    if store::close_if_idle(&mut c, now).await?.is_some() {
        ops::flush_queued(&st.db, &st.catalogue, today).await?;
    }
    let settings = store::settings(&mut c).await?;
    let lifts = store::list_lifts(&mut c, false).await?;
    let plan = store::current_plan(&mut c).await?.map(|v| store::plan_dto(&v, &lifts, today));
    let today_info = store::today(plan.as_ref(), today);
    let open_session = store::open_session(&mut c).await?;
    let recent_sessions = store::recent_sessions(&mut c, 12, None).await?;
    let health_week = store::health_days(&mut c, today - Duration::days(6), today).await?;
    let pending_activity = store::pending_activity(&mut c).await?;
    drop(c);
    let connections = providers::connections(&st).await?;
    Ok(Json(Bootstrap { now, settings, lifts, plan, today: today_info, open_session, recent_sessions, health_week, connections, pending_activity }))
}

pub async fn put_settings(_: Owner, State(st): State<AppState>, Json(s): Json<Settings>) -> AppResult<Json<Settings>> {
    if !(15..=900).contains(&s.rest_seconds) {
        return Err(bad("Rest must be between 15 seconds and 15 minutes"));
    }
    if !(800..=8000).contains(&s.kcal_goal) || s.protein_goal_g > 600 {
        return Err(bad("Those goals look off"));
    }
    let mut c = st.db.acquire().await?;
    store::set_setting(&mut c, "app_settings", &s).await?;
    Ok(Json(s))
}

// ---------- lifts ----------

pub async fn add_lift(_: Owner, State(st): State<AppState>, Query(tz): Query<Tz>, Json(b): Json<NewLift>) -> AppResult<impl IntoResponse> {
    let out = ops::submit(&st.db, &Actor::Owner, Op::AddLift { name: b.name, prescription: b.prescription, tags: None, cues: None }, &st.catalogue, tz.today()).await?;
    let mut c = st.db.acquire().await?;
    let lift = store::get_lift(&mut c, out.lift_id.ok_or_else(|| AppError::Internal(anyhow::anyhow!("no lift id")))?).await?;
    Ok((StatusCode::CREATED, Json(lift)))
}

pub async fn patch_lift(_: Owner, State(st): State<AppState>, Query(tz): Query<Tz>, Path(id): Path<Uuid>, Json(p): Json<LiftPatch>) -> AppResult<impl IntoResponse> {
    ops::submit(&st.db, &Actor::Owner, Op::UpdateLift { lift_id: id, patch: p }, &st.catalogue, tz.today()).await?;
    let mut c = st.db.acquire().await?;
    Ok(Json(store::get_lift(&mut c, id).await?))
}

pub async fn add_clip(_: Owner, State(st): State<AppState>, Query(tz): Query<Tz>, Path(id): Path<Uuid>, Json(b): Json<NewClip>) -> AppResult<impl IntoResponse> {
    ops::submit(&st.db, &Actor::Owner, Op::AddClip { lift_id: id, url: b.url, title: b.title }, &st.catalogue, tz.today()).await?;
    let mut c = st.db.acquire().await?;
    Ok((StatusCode::CREATED, Json(store::get_lift(&mut c, id).await?)))
}

pub async fn delete_clip(_: Owner, State(st): State<AppState>, Query(tz): Query<Tz>, Path(id): Path<Uuid>) -> AppResult<impl IntoResponse> {
    ops::submit(&st.db, &Actor::Owner, Op::RemoveClip { clip_id: id }, &st.catalogue, tz.today()).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Re-run automatic sorting on a lift that is not tagged by you.
pub async fn resort_lift(_: Owner, State(st): State<AppState>, Path(id): Path<Uuid>) -> AppResult<impl IntoResponse> {
    let mut c = st.db.acquire().await?;
    let mut lift = store::get_lift(&mut c, id).await?;
    if lift.tag_source == Some(gym_core::TagSource::User) {
        return Err(AppError::Conflict("You tagged this lift yourself, so it is left as it is".into()));
    }
    let cl = gym_core::classify::classify(&lift.name, &st.catalogue);
    lift.tags = cl.tags;
    lift.tag_source = cl.source;
    store::write_lift(&mut c, &lift).await?;
    Ok(Json(lift))
}

pub async fn lift_progress(_: Owner, State(st): State<AppState>, Query(tz): Query<Tz>, Path(id): Path<Uuid>) -> AppResult<Json<LiftProgress>> {
    let mut c = st.db.acquire().await?;
    store::get_lift(&mut c, id).await?;
    let unit = store::settings(&mut c).await?.unit;
    Ok(Json(progress_for(&mut c, id, unit, tz.tz).await?))
}

pub async fn progress_for(c: &mut sqlx::PgConnection, lift_id: Uuid, unit: Unit, tz: i32) -> AppResult<LiftProgress> {
    let rows: Vec<(Uuid, chrono::DateTime<Utc>, i64, i32)> = sqlx::query_as(
        "select s.session_id, w.started_at, s.weight_g, s.reps from sets s join workout_sessions w on w.id = s.session_id
         where s.lift_id = $1 order by w.started_at, s.logged_at",
    )
    .bind(lift_id)
    .fetch_all(&mut *c)
    .await?;
    type Day = (Uuid, NaiveDate, Vec<(Weight, u32)>);
    let mut sessions: Vec<Day> = Vec::new();
    for (sid, started, w, r) in rows {
        match sessions.last_mut() {
            Some(last) if last.0 == sid => last.2.push((Weight(w), r.max(0) as u32)),
            _ => sessions.push((sid, local_date(started, tz), vec![(Weight(w), r.max(0) as u32)])),
        }
    }
    let describe = |sets: &[(Weight, u32)]| -> String {
        // "80 kg × 5, 5, 5, 5" when the weight held; otherwise each set in full.
        if sets.iter().all(|s| s.0 == sets[0].0) {
            format!("{} × {}", sets[0].0.display_with_unit(unit), sets.iter().map(|s| s.1.to_string()).collect::<Vec<_>>().join(", "))
        } else {
            sets.iter().map(|s| format!("{} × {}", s.0.display_with_unit(unit), s.1)).collect::<Vec<_>>().join(", ")
        }
    };
    let points = sessions
        .iter()
        .filter_map(|(_, d, sets)| {
            let best = best_e1rm(sets.iter().copied())?;
            let top = sets.iter().max_by_key(|s| gym_core::progress::e1rm(s.0, s.1)).copied()?;
            Some(ProgressPoint { date: *d, e1rm: best, best_set: format!("{} × {}", top.0.display_with_unit(unit), top.1) })
        })
        .collect();
    let recent = sessions.iter().rev().take(6).map(|(_, d, sets)| (*d, describe(sets))).collect();
    Ok(LiftProgress { lift_id, points, recent })
}

// ---------- plan ----------

async fn plan_now(st: &AppState, today: NaiveDate) -> AppResult<Option<Plan>> {
    let mut c = st.db.acquire().await?;
    let lifts = store::list_lifts(&mut c, false).await?;
    Ok(store::current_plan(&mut c).await?.map(|v| store::plan_dto(&v, &lifts, today)))
}

pub async fn switch_split(_: Owner, State(st): State<AppState>, Query(tz): Query<Tz>, Json(b): Json<SwitchSplit>) -> AppResult<Json<Option<Plan>>> {
    ops::submit(&st.db, &Actor::Owner, Op::SwitchSplit { key: b.key, split: b.split }, &st.catalogue, tz.today()).await?;
    Ok(Json(plan_now(&st, tz.today()).await?))
}

/// Go back to the plan version before this one.
pub async fn undo_plan(_: Owner, State(st): State<AppState>, Query(tz): Query<Tz>) -> AppResult<Json<Option<Plan>>> {
    let mut c = st.db.acquire().await?;
    let cur = store::current_plan(&mut c).await?.ok_or_else(|| bad("There is no plan"))?;
    let prev = cur.previous_id.ok_or_else(|| bad("This is the first version of the plan"))?;
    store::set_current_plan(&mut c, prev).await?;
    drop(c);
    Ok(Json(plan_now(&st, tz.today()).await?))
}

#[derive(Deserialize)]
pub struct PlanMeta {
    pub name: String,
    #[serde(default)]
    pub block_weeks: Option<u32>,
}

pub async fn plan_meta(_: Owner, State(st): State<AppState>, Query(tz): Query<Tz>, Json(b): Json<PlanMeta>) -> AppResult<Json<Option<Plan>>> {
    ops::submit(&st.db, &Actor::Owner, Op::SetPlanName { name: b.name, block_weeks: b.block_weeks }, &st.catalogue, tz.today()).await?;
    Ok(Json(plan_now(&st, tz.today()).await?))
}

#[derive(Deserialize)]
pub struct PlanStart {
    #[serde(default)]
    pub starts_on: Option<NaiveDate>,
}

pub async fn plan_start(_: Owner, State(st): State<AppState>, Query(tz): Query<Tz>, Json(b): Json<PlanStart>) -> AppResult<Json<Option<Plan>>> {
    ops::submit(&st.db, &Actor::Owner, Op::SetPlanStart { starts_on: b.starts_on }, &st.catalogue, tz.today()).await?;
    Ok(Json(plan_now(&st, tz.today()).await?))
}

/// The lifts from the screens, so a new install has something to train from.
pub fn starter() -> WritePlan {
    let l = |name: &str, sets, reps, weight: f64, cues: &[&str]| PlanLift {
        name: name.into(),
        sets,
        reps,
        weight,
        unit: Some(Unit::Kg),
        rest_seconds: None,
        tags: None,
        cues: if cues.is_empty() { None } else { Some(cues.iter().map(|s| s.to_string()).collect()) },
    };
    WritePlan {
        name: Some("Strength base".into()),
        split_key: Some("ppl".into()),
        split: None,
        block_weeks: Some(6),
        starts_on: None,
        retire_others: false,
        lifts: vec![
            l("Bench press", 4, 5, 80.0, &["Shoulder blades back and down before you unrack", "Bar touches low on the chest", "Feet flat, push the floor away"]),
            l("Overhead press", 3, 8, 47.5, &["Squeeze glutes, ribs down", "Head through once the bar passes"]),
            l("Incline dumbbell press", 3, 10, 26.0, &[]),
            l("Cable fly", 3, 12, 18.0, &[]),
            l("Triceps pushdown", 3, 12, 32.5, &[]),
            l("Lateral raise", 3, 15, 8.0, &[]),
            l("Barbell row", 4, 6, 70.0, &["Hinge to 45 degrees", "Pull to the belly button"]),
            l("Pull-up", 4, 6, 0.0, &[]),
            l("Face pull", 3, 15, 20.0, &[]),
            l("Barbell curl", 3, 10, 30.0, &[]),
            l("Back squat", 4, 5, 100.0, &["Brace before you descend", "Knees track over toes"]),
            l("Romanian deadlift", 3, 8, 90.0, &["Push the hips back, soft knees"]),
            l("Leg press", 3, 10, 160.0, &[]),
            l("Calf raise", 4, 12, 60.0, &[]),
        ],
    }
}

pub async fn load_starter(_: Owner, State(st): State<AppState>, Query(tz): Query<Tz>) -> AppResult<Json<Option<Plan>>> {
    let mut c = st.db.acquire().await?;
    if store::current_plan(&mut c).await?.is_some() {
        return Err(AppError::Conflict("You already have a plan".into()));
    }
    drop(c);
    ops::submit(&st.db, &Actor::Owner, Op::WritePlan(starter()), &st.catalogue, tz.today()).await?;
    Ok(Json(plan_now(&st, tz.today()).await?))
}

// ---------- sets and sessions ----------

/// The outbox from the phone: new sessions, sets, deletions and End taps, in one go.
pub async fn sync(_: Owner, State(st): State<AppState>, Query(tz): Query<Tz>, Json(b): Json<SyncSets>) -> AppResult<Json<SyncResult>> {
    if b.sets.len() > 500 || b.sessions.len() > 50 {
        return Err(bad("Send at most 500 sets at a time"));
    }
    let mut tx = st.db.begin().await?;
    let mut touched: Vec<Uuid> = Vec::new();
    for s in &b.sessions {
        store::ensure_session(&mut tx, s).await?;
        touched.push(s.id);
    }
    let lift_ids: Vec<(Uuid,)> = sqlx::query_as("select id from lifts").fetch_all(&mut *tx).await?;
    let mut accepted = Vec::new();
    for s in &b.sets {
        if !lift_ids.iter().any(|(id,)| *id == s.lift_id) {
            return Err(bad("A set points at a lift that does not exist"));
        }
        let known: Option<(Uuid,)> = sqlx::query_as("select id from workout_sessions where id = $1").bind(s.session_id).fetch_optional(&mut *tx).await?;
        if known.is_none() {
            return Err(bad("A set points at a session that does not exist. Send the session first"));
        }
        store::upsert_set(&mut tx, s).await?;
        if store::reopen_if_idle_guess(&mut tx, s.session_id, s.logged_at).await? {
            tracing::info!(session = %s.session_id, "reopened: sets arrived after an idle close");
        }
        accepted.push(s.id);
        touched.push(s.session_id);
    }
    for id in &b.deleted {
        store::delete_set(&mut tx, *id).await?;
    }
    let mut closed_any = false;
    for e in &b.ended {
        let s = store::get_session(&mut tx, e.id).await?;
        let w = SessionWindow { started_at: s.started_at, last_set_at: s.sets.iter().map(|x| x.logged_at).max(), closed_at: s.closed_at };
        if let Decision::Close { at, reason, .. } = decide(&w, &Signal::Tap { at: e.at }) {
            closed_any |= store::close_session(&mut tx, s.id, at, reason).await?;
        }
        touched.push(e.id);
    }
    touched.sort();
    touched.dedup();
    let mut sessions = Vec::new();
    for id in touched {
        sessions.push(store::get_session(&mut tx, id).await?);
    }
    tx.commit().await?;
    if closed_any {
        ops::flush_queued(&st.db, &st.catalogue, tz.today()).await?;
    }
    Ok(Json(SyncResult { accepted_sets: accepted, deleted: b.deleted, sessions }))
}

#[derive(Deserialize)]
pub struct Page {
    #[serde(default)]
    pub before: Option<chrono::DateTime<Utc>>,
    #[serde(default)]
    pub limit: Option<i64>,
}

pub async fn sessions(_: Owner, State(st): State<AppState>, Query(p): Query<Page>) -> AppResult<Json<Vec<Session>>> {
    let mut c = st.db.acquire().await?;
    Ok(Json(store::recent_sessions(&mut c, p.limit.unwrap_or(20), p.before).await?))
}

pub async fn session(_: Owner, State(st): State<AppState>, Path(id): Path<Uuid>) -> AppResult<Json<Session>> {
    let mut c = st.db.acquire().await?;
    Ok(Json(store::get_session(&mut c, id).await?))
}

#[derive(Deserialize)]
pub struct Days {
    #[serde(default)]
    pub days: Option<i64>,
    #[serde(default)]
    pub tz: i32,
}

pub async fn health(_: Owner, State(st): State<AppState>, Query(q): Query<Days>) -> AppResult<Json<Vec<HealthDay>>> {
    let today = local_date(Utc::now(), q.tz);
    let mut c = st.db.acquire().await?;
    Ok(Json(store::health_days(&mut c, today - Duration::days(q.days.unwrap_or(7).clamp(1, 3660) - 1), today).await?))
}

// ---------- agents ----------

#[derive(sqlx::FromRow)]
struct AgentRow {
    id: Uuid,
    name: String,
    key_prefix: String,
    created_at: chrono::DateTime<Utc>,
    last_used_at: Option<chrono::DateTime<Utc>>,
    ask_first: bool,
    revoked_at: Option<chrono::DateTime<Utc>>,
}

impl From<AgentRow> for Agent {
    fn from(r: AgentRow) -> Agent {
        Agent { id: r.id, name: r.name, key_prefix: r.key_prefix, created_at: r.created_at, last_used_at: r.last_used_at, ask_first: r.ask_first, revoked: r.revoked_at.is_some() }
    }
}

pub async fn agents(_: Owner, State(st): State<AppState>) -> AppResult<Json<Vec<Agent>>> {
    let rows: Vec<AgentRow> =
        sqlx::query_as("select id, name, key_prefix, created_at, last_used_at, ask_first, revoked_at from agents order by revoked_at nulls first, created_at").fetch_all(&st.db).await?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

#[derive(Deserialize)]
pub struct AgentPatch {
    #[serde(default)]
    pub ask_first: Option<bool>,
    #[serde(default)]
    pub revoke: Option<bool>,
}

pub async fn patch_agent(_: Owner, State(st): State<AppState>, Path(id): Path<Uuid>, Json(b): Json<AgentPatch>) -> AppResult<Json<Agent>> {
    if let Some(a) = b.ask_first {
        sqlx::query("update agents set ask_first = $2 where id = $1").bind(id).bind(a).execute(&st.db).await?;
    }
    if b.revoke == Some(true) {
        sqlx::query("update agents set revoked_at = coalesce(revoked_at, now()) where id = $1").bind(id).execute(&st.db).await?;
        store::decline_waiting_from(&st.db, id).await?;
    }
    let row: AgentRow = sqlx::query_as("select id, name, key_prefix, created_at, last_used_at, ask_first, revoked_at from agents where id = $1")
        .bind(id)
        .fetch_optional(&st.db)
        .await?
        .ok_or_else(|| not_found("No such agent"))?;
    Ok(Json(row.into()))
}

pub async fn activity(_: Owner, State(st): State<AppState>) -> AppResult<Json<Vec<Activity>>> {
    Ok(Json(ops::list_activity(&st.db, 60).await?))
}

pub async fn activity_action(_: Owner, State(st): State<AppState>, Query(tz): Query<Tz>, Path((id, action)): Path<(Uuid, String)>) -> AppResult<StatusCode> {
    match action.as_str() {
        "undo" => ops::undo_activity(&st.db, id).await?,
        "decline" => ops::decline_activity(&st.db, id).await?,
        "accept" => {
            ops::accept_activity(&st.db, id, &st.catalogue, tz.today()).await?;
        }
        _ => return Err(not_found("Use undo, accept or decline")),
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------- export ----------

/// Everything you own, as one JSON file.
pub async fn export(_: Owner, State(st): State<AppState>) -> AppResult<impl IntoResponse> {
    let mut c = st.db.acquire().await?;
    let lifts = store::list_lifts(&mut c, true).await?;
    let sessions = store::all_sessions(&mut c).await?;
    let open = store::open_session(&mut c).await?;
    let health = store::health_days(&mut c, NaiveDate::from_ymd_opt(2000, 1, 1).expect("date"), NaiveDate::from_ymd_opt(2100, 1, 1).expect("date")).await?;
    let plans: Vec<(Uuid, chrono::DateTime<Utc>, String, SqlJson<serde_json::Value>)> =
        sqlx::query_as("select id, created_at, created_by, data from plan_versions order by created_at").fetch_all(&mut *c).await?;
    let settings = store::settings(&mut c).await?;
    let body = serde_json::json!({
        "exported_at": Utc::now(),
        "settings": settings,
        "lifts": lifts,
        "plans": plans.into_iter().map(|(id, at, by, SqlJson(d))| serde_json::json!({"id": id, "created_at": at, "created_by": by, "data": d})).collect::<Vec<_>>(),
        "sessions": open.into_iter().chain(sessions).collect::<Vec<_>>(),
        "health_days": health,
    });
    let name = format!("attachment; filename=\"plate-export-{}.json\"", Utc::now().format("%Y-%m-%d"));
    Ok(([(header::CONTENT_TYPE, "application/json".to_string()), (header::CONTENT_DISPOSITION, name)], serde_json::to_string_pretty(&body)?))
}

pub fn default_rx() -> Prescription {
    Prescription::default()
}
