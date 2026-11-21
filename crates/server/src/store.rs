//! Reading and writing the database. No permission checks here: callers decide who
//! may do what (see `ops` for writes made on someone's behalf).

use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use gym_core::api::{self, Clip, HealthDay, Lift, Plan, PlanDay, Prescription, Session, SetEntry, Settings, Today, WorkoutNumbers};
use gym_core::classify::normalise;
use gym_core::session::{CloseReason, Decision, SessionWindow, Signal, decide};
use gym_core::split::{PoolLift, Split, deal};
use gym_core::{LiftTags, TagSource, Unit, Weight};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sqlx::types::Json;
use sqlx::{FromRow, PgConnection};
use uuid::Uuid;

use crate::clips;
use crate::error::{AppError, AppResult, not_found};

// ---------- settings ----------

pub async fn get_setting<T: DeserializeOwned>(c: &mut PgConnection, key: &str) -> AppResult<Option<T>> {
    let row: Option<(Json<serde_json::Value>,)> = sqlx::query_as("select value from settings where key = $1").bind(key).fetch_optional(&mut *c).await?;
    Ok(match row {
        Some((Json(v),)) => serde_json::from_value(v).ok(),
        None => None,
    })
}

pub async fn set_setting<T: Serialize>(c: &mut PgConnection, key: &str, value: &T) -> AppResult<()> {
    sqlx::query("insert into settings (key, value) values ($1, $2) on conflict (key) do update set value = excluded.value")
        .bind(key)
        .bind(Json(serde_json::to_value(value)?))
        .execute(&mut *c)
        .await?;
    Ok(())
}

pub async fn settings(c: &mut PgConnection) -> AppResult<Settings> {
    Ok(get_setting(c, "app_settings").await?.unwrap_or_default())
}

// ---------- lifts ----------

#[derive(FromRow)]
struct LiftRow {
    id: Uuid,
    name: String,
    tags: Option<Json<LiftTags>>,
    tag_source: Option<String>,
    sets: i32,
    reps: i32,
    weight_g: i64,
    rest_s: Option<i32>,
    cues: Json<Vec<String>>,
    archived: bool,
}

#[derive(FromRow)]
struct ClipRow {
    id: Uuid,
    lift_id: Uuid,
    url: String,
    provider: String,
    title: String,
    added_by: String,
    position: i32,
}

impl From<ClipRow> for Clip {
    fn from(r: ClipRow) -> Clip {
        Clip {
            embed_url: clips::embed_for(&r.url),
            id: r.id,
            lift_id: r.lift_id,
            url: r.url,
            provider: clips::provider_parse(&r.provider),
            title: r.title,
            added_by: r.added_by,
            position: r.position,
        }
    }
}

fn lift_from(r: LiftRow, clips: Vec<Clip>) -> Lift {
    Lift {
        id: r.id,
        name: r.name,
        tags: r.tags.map(|t| t.0),
        tag_source: r.tag_source.as_deref().and_then(TagSource::parse),
        prescription: Prescription { sets: r.sets.max(0) as u32, reps: r.reps.max(0) as u32, weight: Weight(r.weight_g), rest_seconds: r.rest_s.map(|s| s.max(0) as u32) },
        cues: r.cues.0,
        clips,
        archived: r.archived,
    }
}

const LIFT_COLS: &str = "id, name, tags, tag_source, sets, reps, weight_g, rest_s, cues, archived";

pub async fn list_lifts(c: &mut PgConnection, include_archived: bool) -> AppResult<Vec<Lift>> {
    let sql = format!("select {LIFT_COLS} from lifts where ($1 or not archived) order by position, created_at");
    let rows: Vec<LiftRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(include_archived).fetch_all(&mut *c).await?;
    let clip_rows: Vec<ClipRow> =
        sqlx::query_as("select id, lift_id, url, provider, title, added_by, position from clips order by position, created_at").fetch_all(&mut *c).await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let id = r.id;
            let clips = clip_rows.iter().filter(|cr| cr.lift_id == id).map(|cr| {
                ClipRow { id: cr.id, lift_id: cr.lift_id, url: cr.url.clone(), provider: cr.provider.clone(), title: cr.title.clone(), added_by: cr.added_by.clone(), position: cr.position }.into()
            });
            lift_from(r, clips.collect())
        })
        .collect())
}

pub async fn get_lift(c: &mut PgConnection, id: Uuid) -> AppResult<Lift> {
    let sql = format!("select {LIFT_COLS} from lifts where id = $1");
    let row: LiftRow = sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(id).fetch_optional(&mut *c).await?.ok_or_else(|| not_found("No lift with that id"))?;
    let clips: Vec<ClipRow> =
        sqlx::query_as("select id, lift_id, url, provider, title, added_by, position from clips where lift_id = $1 order by position, created_at").bind(id).fetch_all(&mut *c).await?;
    Ok(lift_from(row, clips.into_iter().map(Into::into).collect()))
}

pub async fn find_lift_by_name(c: &mut PgConnection, name: &str) -> AppResult<Option<Uuid>> {
    let row: Option<(Uuid,)> = sqlx::query_as("select id from lifts where norm_name = $1 and not archived").bind(normalise(name)).fetch_optional(&mut *c).await?;
    Ok(row.map(|r| r.0))
}

pub struct NewLiftRow<'a> {
    pub name: &'a str,
    pub tags: Option<&'a LiftTags>,
    pub tag_source: Option<TagSource>,
    pub prescription: &'a Prescription,
    pub cues: &'a [String],
    pub created_by: &'a str,
}

pub fn check_name(name: &str) -> AppResult<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest("The lift needs a name".into()));
    }
    if name.chars().count() > 80 {
        return Err(AppError::BadRequest("Keep the name under 80 characters".into()));
    }
    if normalise(name).is_empty() {
        return Err(AppError::BadRequest("The name needs letters or numbers".into()));
    }
    Ok(name.to_string())
}

pub fn check_prescription(p: &Prescription) -> AppResult<()> {
    if p.sets == 0 || p.sets > 20 {
        return Err(AppError::BadRequest("Sets must be between 1 and 20".into()));
    }
    if p.reps == 0 || p.reps > 100 {
        return Err(AppError::BadRequest("Reps must be between 1 and 100".into()));
    }
    if p.weight.0 < 0 || p.weight.0 > 1_000_000 {
        return Err(AppError::BadRequest("Weight must be between 0 and 1000 kg".into()));
    }
    if let Some(r) = p.rest_seconds
        && !(10..=900).contains(&r)
    {
        return Err(AppError::BadRequest("Rest must be between 10 seconds and 15 minutes".into()));
    }
    Ok(())
}

pub fn check_cues(cues: &[String]) -> AppResult<Vec<String>> {
    if cues.len() > 8 {
        return Err(AppError::BadRequest("Keep it to eight cues".into()));
    }
    let out: Vec<String> = cues.iter().map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).collect();
    if out.iter().any(|c| c.chars().count() > 160) {
        return Err(AppError::BadRequest("Keep each cue under 160 characters".into()));
    }
    Ok(out)
}

pub async fn insert_lift(c: &mut PgConnection, l: NewLiftRow<'_>) -> AppResult<Uuid> {
    let name = check_name(l.name)?;
    check_prescription(l.prescription)?;
    let cues = check_cues(l.cues)?;
    if find_lift_by_name(c, &name).await?.is_some() {
        return Err(AppError::Conflict(format!("{name} is already in your lifts")));
    }
    let id = Uuid::new_v4();
    let position: (Option<i32>,) = sqlx::query_as("select max(position) from lifts").fetch_one(&mut *c).await?;
    sqlx::query(
        "insert into lifts (id, name, norm_name, tags, tag_source, sets, reps, weight_g, rest_s, cues, position, created_by)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(id)
    .bind(&name)
    .bind(normalise(&name))
    .bind(l.tags.map(|t| Json(t.clone())))
    .bind(l.tag_source.map(|s| s.as_str()))
    .bind(l.prescription.sets as i32)
    .bind(l.prescription.reps as i32)
    .bind(l.prescription.weight.0)
    .bind(l.prescription.rest_seconds.map(|r| r as i32))
    .bind(Json(cues))
    .bind(position.0.unwrap_or(0) + 1)
    .bind(l.created_by)
    .execute(&mut *c)
    .await?;
    Ok(id)
}

/// Write a lift back exactly as given (used by edits and by undo).
pub async fn write_lift(c: &mut PgConnection, l: &Lift) -> AppResult<()> {
    let name = check_name(&l.name)?;
    check_prescription(&l.prescription)?;
    let cues = check_cues(&l.cues)?;
    if !l.archived
        && let Some(other) = find_lift_by_name(c, &name).await?
        && other != l.id
    {
        return Err(AppError::Conflict(format!("{name} is already in your lifts")));
    }
    let done = sqlx::query(
        "update lifts set name = $2, norm_name = $3, tags = $4, tag_source = $5, sets = $6, reps = $7, weight_g = $8, rest_s = $9, cues = $10, archived = $11
         where id = $1",
    )
    .bind(l.id)
    .bind(&name)
    .bind(normalise(&name))
    .bind(l.tags.as_ref().map(|t| Json(t.clone())))
    .bind(l.tag_source.map(|s| s.as_str()))
    .bind(l.prescription.sets as i32)
    .bind(l.prescription.reps as i32)
    .bind(l.prescription.weight.0)
    .bind(l.prescription.rest_seconds.map(|r| r as i32))
    .bind(Json(cues))
    .bind(l.archived)
    .execute(&mut *c)
    .await?;
    if done.rows_affected() == 0 {
        return Err(not_found("No lift with that id"));
    }
    Ok(())
}

pub async fn insert_clip(c: &mut PgConnection, lift_id: Uuid, url: &str, title: Option<&str>, added_by: &str) -> AppResult<Clip> {
    let parsed = clips::parse(url).map_err(AppError::BadRequest)?;
    let lift = get_lift(c, lift_id).await?;
    if lift.clips.len() >= 12 {
        return Err(AppError::BadRequest("A lift can hold twelve clips".into()));
    }
    if lift.clips.iter().any(|cl| cl.url == parsed.url) {
        return Err(AppError::Conflict("That clip is already on this lift".into()));
    }
    let title = title.map(str::trim).filter(|t| !t.is_empty()).map(|t| t.chars().take(80).collect::<String>()).unwrap_or_else(|| match parsed.provider {
        api::ClipProvider::Youtube => format!("{} on YouTube", lift.name),
        api::ClipProvider::Instagram => format!("{} reel", lift.name),
        api::ClipProvider::Other => format!("{} link", lift.name),
    });
    let id = Uuid::new_v4();
    let position = lift.clips.iter().map(|c| c.position).max().unwrap_or(-1) + 1;
    sqlx::query("insert into clips (id, lift_id, url, provider, title, added_by, position) values ($1, $2, $3, $4, $5, $6, $7)")
        .bind(id)
        .bind(lift_id)
        .bind(&parsed.url)
        .bind(clips::provider_str(parsed.provider))
        .bind(&title)
        .bind(added_by)
        .bind(position)
        .execute(&mut *c)
        .await?;
    Ok(Clip { id, lift_id, embed_url: parsed.embed_url, url: parsed.url, provider: parsed.provider, title, added_by: added_by.to_string(), position })
}

pub async fn restore_clip(c: &mut PgConnection, clip: &Clip) -> AppResult<()> {
    sqlx::query("insert into clips (id, lift_id, url, provider, title, added_by, position) values ($1, $2, $3, $4, $5, $6, $7) on conflict (id) do nothing")
        .bind(clip.id)
        .bind(clip.lift_id)
        .bind(&clip.url)
        .bind(clips::provider_str(clip.provider))
        .bind(&clip.title)
        .bind(&clip.added_by)
        .bind(clip.position)
        .execute(&mut *c)
        .await?;
    Ok(())
}

pub async fn get_clip(c: &mut PgConnection, id: Uuid) -> AppResult<Clip> {
    let row: ClipRow = sqlx::query_as("select id, lift_id, url, provider, title, added_by, position from clips where id = $1")
        .bind(id)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| not_found("No clip with that id"))?;
    Ok(row.into())
}

pub async fn delete_clip(c: &mut PgConnection, id: Uuid) -> AppResult<()> {
    sqlx::query("delete from clips where id = $1").bind(id).execute(&mut *c).await?;
    Ok(())
}

// ---------- plans ----------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlanData {
    pub name: String,
    pub split: Split,
    #[serde(default)]
    pub block_weeks: Option<u32>,
    #[serde(default)]
    pub block_start: Option<NaiveDate>,
    /// First day of the plan; weeks count from here and Today waits for it.
    #[serde(default)]
    pub starts_on: Option<NaiveDate>,
}

#[derive(Debug, Clone)]
pub struct PlanVersion {
    pub id: Uuid,
    pub previous_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
    pub data: PlanData,
}

#[derive(FromRow)]
struct PlanRow {
    id: Uuid,
    previous_id: Option<Uuid>,
    created_at: DateTime<Utc>,
    created_by: String,
    data: Json<PlanData>,
}

pub async fn plan_version(c: &mut PgConnection, id: Uuid) -> AppResult<PlanVersion> {
    let r: PlanRow = sqlx::query_as("select id, previous_id, created_at, created_by, data from plan_versions where id = $1")
        .bind(id)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| not_found("No plan version with that id"))?;
    Ok(PlanVersion { id: r.id, previous_id: r.previous_id, created_at: r.created_at, created_by: r.created_by, data: r.data.0 })
}

pub async fn current_plan(c: &mut PgConnection) -> AppResult<Option<PlanVersion>> {
    match get_setting::<Uuid>(c, "current_plan").await? {
        Some(id) => Ok(Some(plan_version(c, id).await?)),
        None => Ok(None),
    }
}

/// Store a new version on top of the current one and make it current.
pub async fn push_plan(c: &mut PgConnection, data: &PlanData, created_by: &str) -> AppResult<Uuid> {
    data.split.validate().map_err(AppError::BadRequest)?;
    if data.name.trim().is_empty() || data.name.chars().count() > 60 {
        return Err(AppError::BadRequest("The plan name must be 1 to 60 characters".into()));
    }
    if let Some(w) = data.block_weeks
        && !(1..=52).contains(&w)
    {
        return Err(AppError::BadRequest("A block runs 1 to 52 weeks".into()));
    }
    let previous = get_setting::<Uuid>(c, "current_plan").await?;
    let id = Uuid::new_v4();
    sqlx::query("insert into plan_versions (id, previous_id, created_by, data) values ($1, $2, $3, $4)")
        .bind(id)
        .bind(previous)
        .bind(created_by)
        .bind(Json(data))
        .execute(&mut *c)
        .await?;
    set_setting(c, "current_plan", &id).await?;
    Ok(id)
}

/// Point "current" at an older version. History is kept.
pub async fn set_current_plan(c: &mut PgConnection, id: Uuid) -> AppResult<()> {
    plan_version(c, id).await?;
    set_setting(c, "current_plan", &id).await
}

pub fn local_date(now: DateTime<Utc>, tz_offset_min: i32) -> NaiveDate {
    (now - Duration::minutes(tz_offset_min as i64)).date_naive()
}

pub fn plan_dto(v: &PlanVersion, lifts: &[Lift], today: NaiveDate) -> Plan {
    let active: Vec<&Lift> = lifts.iter().filter(|l| !l.archived).collect();
    let pool: Vec<PoolLift> = active.iter().map(|l| PoolLift { id: l.id, tags: l.tags.as_ref() }).collect();
    let dealt = deal(&v.data.split, &pool);
    let block_week = match (v.data.block_start.max(v.data.starts_on), v.data.block_weeks) {
        (Some(start), Some(weeks)) => Some((((today - start).num_days().max(0) / 7) as u32 + 1).min(weeks)),
        _ => None,
    };
    Plan {
        version_id: v.id,
        created_at: v.created_at,
        created_by: v.created_by.clone(),
        name: v.data.name.clone(),
        block_week,
        block_weeks: v.data.block_weeks,
        starts_on: v.data.starts_on,
        days: v
            .data
            .split
            .days
            .iter()
            .zip(dealt.days)
            .map(|(d, lifts)| PlanDay { name: d.name.clone(), weekday: d.weekday, lifts })
            .collect(),
        unplaced: dealt.unplaced,
        split: v.data.split.clone(),
        can_undo: v.previous_id.is_some(),
    }
}

pub fn today(plan: Option<&Plan>, date: NaiveDate) -> Today {
    let wd = date.weekday().num_days_from_monday() as u8;
    let Some(plan) = plan else { return Today { day_index: None, is_rest_day: true, next_day_index: None } };
    // Not started yet: nothing today, and "next" is the first training day on or after the start.
    if let Some(start) = plan.starts_on.filter(|s| *s > date) {
        let swd = start.weekday().num_days_from_monday() as u8;
        let next = (0..7u8).map(|k| (swd + k) % 7).find_map(|w| plan.days.iter().position(|d| d.weekday == w));
        return Today { day_index: None, is_rest_day: true, next_day_index: next };
    }
    let idx = plan.days.iter().position(|d| d.weekday == wd);
    let next = (1..=7u8).map(|k| (wd + k) % 7).find_map(|w| plan.days.iter().position(|d| d.weekday == w));
    Today { day_index: idx, is_rest_day: idx.is_none(), next_day_index: next }
}

// ---------- sessions and sets ----------

#[derive(FromRow)]
struct SessionRow {
    id: Uuid,
    started_at: DateTime<Utc>,
    closed_at: Option<DateTime<Utc>>,
    close_reason: Option<String>,
    day_name: String,
    day_index: Option<i32>,
    workout: Option<Json<WorkoutNumbers>>,
}

#[derive(FromRow)]
struct SetRow {
    id: Uuid,
    session_id: Uuid,
    lift_id: Uuid,
    weight_g: i64,
    reps: i32,
    unit: String,
    logged_at: DateTime<Utc>,
}

const SESSION_COLS: &str = "id, started_at, closed_at, close_reason, day_name, day_index, last_set_at, workout";

async fn hydrate(c: &mut PgConnection, rows: Vec<SessionRow>) -> AppResult<Vec<Session>> {
    let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    let sets: Vec<SetRow> = sqlx::query_as("select id, session_id, lift_id, weight_g, reps, unit, logged_at from sets where session_id = any($1) order by logged_at, created_at")
        .bind(&ids)
        .fetch_all(&mut *c)
        .await?;
    Ok(rows
        .into_iter()
        .map(|r| Session {
            id: r.id,
            started_at: r.started_at,
            closed_at: r.closed_at,
            close_reason: r.close_reason.as_deref().and_then(CloseReason::parse),
            day_name: r.day_name,
            day_index: r.day_index.map(|d| d.max(0) as u32),
            workout: r.workout.map(|w| w.0),
            sets: sets
                .iter()
                .filter(|s| s.session_id == r.id)
                .map(|s| SetEntry {
                    id: s.id,
                    session_id: s.session_id,
                    lift_id: s.lift_id,
                    weight: Weight(s.weight_g),
                    reps: s.reps.max(0) as u32,
                    unit: Unit::parse(&s.unit).unwrap_or_default(),
                    logged_at: s.logged_at,
                })
                .collect(),
        })
        .collect())
}

pub async fn get_session(c: &mut PgConnection, id: Uuid) -> AppResult<Session> {
    let sql = format!("select {SESSION_COLS} from workout_sessions where id = $1");
    let row: SessionRow = sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(id).fetch_optional(&mut *c).await?.ok_or_else(|| not_found("No session with that id"))?;
    Ok(hydrate(c, vec![row]).await?.remove(0))
}

pub async fn open_session(c: &mut PgConnection) -> AppResult<Option<Session>> {
    let sql = format!("select {SESSION_COLS} from workout_sessions where closed_at is null limit 1");
    let row: Option<SessionRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql)).fetch_optional(&mut *c).await?;
    match row {
        Some(r) => Ok(Some(hydrate(c, vec![r]).await?.remove(0))),
        None => Ok(None),
    }
}

/// A revoked agent's queued and proposed changes must never apply later.
pub async fn decline_waiting_from(db: &sqlx::PgPool, agent: Uuid) -> AppResult<u64> {
    let r = sqlx::query("update activity set status = 'declined', resolved_at = now() where agent_id = $1 and status in ('queued', 'proposed')").bind(agent).execute(db).await?;
    Ok(r.rows_affected())
}

pub async fn recent_sessions(c: &mut PgConnection, limit: i64, before: Option<DateTime<Utc>>) -> AppResult<Vec<Session>> {
    sessions_page(c, limit.clamp(1, 200), before).await
}

/// Every finished session, for the backup export.
pub async fn all_sessions(c: &mut PgConnection) -> AppResult<Vec<Session>> {
    sessions_page(c, i64::MAX, None).await
}

async fn sessions_page(c: &mut PgConnection, limit: i64, before: Option<DateTime<Utc>>) -> AppResult<Vec<Session>> {
    let sql = format!(
        "select {SESSION_COLS} from workout_sessions where closed_at is not null and ($2::timestamptz is null or started_at < $2) order by started_at desc limit $1"
    );
    let rows: Vec<SessionRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(limit).bind(before).fetch_all(&mut *c).await?;
    hydrate(c, rows).await
}

fn window(s: &Session) -> SessionWindow {
    SessionWindow { started_at: s.started_at, last_set_at: s.sets.iter().map(|x| x.logged_at).max(), closed_at: s.closed_at }
}

/// Insert a session if it is new. A new session closes any other open one first.
pub async fn ensure_session(c: &mut PgConnection, s: &api::StartSession) -> AppResult<()> {
    let exists: Option<(Uuid,)> = sqlx::query_as("select id from workout_sessions where id = $1").bind(s.id).fetch_optional(&mut *c).await?;
    if exists.is_some() {
        return Ok(());
    }
    if s.day_name.trim().is_empty() || s.day_name.chars().count() > 60 {
        return Err(AppError::BadRequest("The session needs a day name".into()));
    }
    if let Some(open) = open_session(c).await? {
        let at = window(&open).last_activity();
        close_session(c, open.id, at, CloseReason::Tap).await?;
    }
    sqlx::query("insert into workout_sessions (id, started_at, day_name, day_index) values ($1, $2, $3, $4)")
        .bind(s.id)
        .bind(s.started_at)
        .bind(s.day_name.trim())
        .bind(s.day_index.map(|d| d as i32))
        .execute(&mut *c)
        .await?;
    Ok(())
}

/// Upsert a set from the phone. The id is made on the phone, so a retry is a no-op
/// and an edit replaces the old values.
pub async fn upsert_set(c: &mut PgConnection, s: &SetEntry) -> AppResult<()> {
    if s.reps > 200 || s.weight.0 < 0 || s.weight.0 > 1_000_000 {
        return Err(AppError::BadRequest("That set has impossible numbers".into()));
    }
    sqlx::query(
        "insert into sets (id, session_id, lift_id, weight_g, reps, unit, logged_at) values ($1, $2, $3, $4, $5, $6, $7)
         on conflict (id) do update set weight_g = excluded.weight_g, reps = excluded.reps, unit = excluded.unit",
    )
    .bind(s.id)
    .bind(s.session_id)
    .bind(s.lift_id)
    .bind(s.weight.0)
    .bind(s.reps as i32)
    .bind(s.unit.label())
    .bind(s.logged_at)
    .execute(&mut *c)
    .await?;
    sqlx::query("update workout_sessions set last_set_at = greatest(coalesce(last_set_at, $2), $2) where id = $1")
        .bind(s.session_id)
        .bind(s.logged_at)
        .execute(&mut *c)
        .await?;
    Ok(())
}

pub async fn delete_set(c: &mut PgConnection, id: Uuid) -> AppResult<()> {
    sqlx::query("delete from sets where id = $1").bind(id).execute(&mut *c).await?;
    Ok(())
}

/// Close a session. Only an open session closes; a second close does nothing.
/// Returns true if this call closed it.
pub async fn close_session(c: &mut PgConnection, id: Uuid, at: DateTime<Utc>, reason: CloseReason) -> AppResult<bool> {
    let done = sqlx::query("update workout_sessions set closed_at = $2, close_reason = $3 where id = $1 and closed_at is null")
        .bind(id)
        .bind(at)
        .bind(reason.as_str())
        .execute(&mut *c)
        .await?;
    Ok(done.rows_affected() == 1)
}

/// The 90-minute rule only guesses that a session is over. If a set logged later than
/// that close arrives (the phone was offline), the session was still going: reopen it,
/// unless another session has started since.
pub async fn reopen_if_idle_guess(c: &mut PgConnection, session_id: Uuid, logged_at: DateTime<Utc>) -> AppResult<bool> {
    let row: Option<(Option<DateTime<Utc>>, Option<String>)> =
        sqlx::query_as("select closed_at, close_reason from workout_sessions where id = $1").bind(session_id).fetch_optional(&mut *c).await?;
    let Some((Some(closed_at), Some(reason))) = row else { return Ok(false) };
    if reason != CloseReason::Idle.as_str() || logged_at <= closed_at {
        return Ok(false);
    }
    let other_open: Option<(Uuid,)> = sqlx::query_as("select id from workout_sessions where closed_at is null and id <> $1").bind(session_id).fetch_optional(&mut *c).await?;
    if other_open.is_some() {
        // A newer session is running; extend this one's end to its last set instead.
        sqlx::query("update workout_sessions set closed_at = $2 where id = $1").bind(session_id).bind(logged_at).execute(&mut *c).await?;
        return Ok(false);
    }
    sqlx::query("update workout_sessions set closed_at = null, close_reason = null where id = $1").bind(session_id).execute(&mut *c).await?;
    Ok(true)
}

pub async fn attach_workout(c: &mut PgConnection, session_id: Uuid, numbers: &WorkoutNumbers) -> AppResult<()> {
    sqlx::query("update workout_sessions set workout = $2 where id = $1").bind(session_id).bind(Json(numbers)).execute(&mut *c).await?;
    Ok(())
}

/// Apply a close signal to whichever session it belongs to.
/// Returns the session that changed and what happened, for logging.
pub async fn apply_signal(c: &mut PgConnection, signal: &Signal) -> AppResult<(Option<Uuid>, &'static str)> {
    // Candidates: the open session, plus recently closed ones for late workout records.
    let mut candidates: Vec<Session> = Vec::new();
    if let Some(open) = open_session(c).await? {
        candidates.push(open);
    }
    if matches!(signal, Signal::WorkoutRecord { .. }) {
        candidates.extend(recent_sessions(c, 10, None).await?);
    }
    for s in candidates {
        match decide(&window(&s), signal) {
            Decision::Close { at, reason, .. } => {
                let closed = close_session(c, s.id, at, reason).await?;
                return Ok((Some(s.id), if closed { "closed" } else { "already closed" }));
            }
            Decision::AttachOnly => return Ok((Some(s.id), "attached")),
            Decision::Ignore => continue,
        }
    }
    Ok((None, "ignored"))
}

/// Close the open session if it has gone 90 minutes without a set.
pub async fn close_if_idle(c: &mut PgConnection, now: DateTime<Utc>) -> AppResult<Option<Uuid>> {
    let Some(open) = open_session(c).await? else { return Ok(None) };
    if let Decision::Close { at, reason, .. } = decide(&window(&open), &Signal::Idle { now }) {
        close_session(c, open.id, at, reason).await?;
        return Ok(Some(open.id));
    }
    Ok(None)
}

// ---------- health ----------

#[derive(FromRow)]
struct HealthRow {
    date: NaiveDate,
    energy_burned_kcal: Option<f64>,
    energy_eaten_kcal: Option<f64>,
    protein_g: Option<f64>,
    carbs_g: Option<f64>,
    fat_g: Option<f64>,
}

pub async fn health_days(c: &mut PgConnection, from: NaiveDate, to: NaiveDate) -> AppResult<Vec<HealthDay>> {
    let rows: Vec<HealthRow> = sqlx::query_as(
        "select date, energy_burned_kcal, energy_eaten_kcal, protein_g, carbs_g, fat_g from health_days where date between $1 and $2 order by date",
    )
    .bind(from)
    .bind(to)
    .fetch_all(&mut *c)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| HealthDay { date: r.date, energy_burned_kcal: r.energy_burned_kcal, energy_eaten_kcal: r.energy_eaten_kcal, protein_g: r.protein_g, carbs_g: r.carbs_g, fat_g: r.fat_g })
        .collect())
}

/// Merge a day's totals. A zero or missing value never replaces a number already
/// stored: Lifesum can wipe old days when its diary is browsed (RESEARCH.md).
pub async fn merge_health_day(c: &mut PgConnection, d: &HealthDay) -> AppResult<()> {
    sqlx::query(
        "insert into health_days (date, energy_burned_kcal, energy_eaten_kcal, protein_g, carbs_g, fat_g, updated_at)
         values ($1, nullif($2, 0), nullif($3, 0), nullif($4, 0), nullif($5, 0), nullif($6, 0), now())
         on conflict (date) do update set
           energy_burned_kcal = coalesce(nullif(excluded.energy_burned_kcal, 0), health_days.energy_burned_kcal),
           energy_eaten_kcal = coalesce(nullif(excluded.energy_eaten_kcal, 0), health_days.energy_eaten_kcal),
           protein_g = coalesce(nullif(excluded.protein_g, 0), health_days.protein_g),
           carbs_g = coalesce(nullif(excluded.carbs_g, 0), health_days.carbs_g),
           fat_g = coalesce(nullif(excluded.fat_g, 0), health_days.fat_g),
           updated_at = now()",
    )
    .bind(d.date)
    .bind(d.energy_burned_kcal)
    .bind(d.energy_eaten_kcal)
    .bind(d.protein_g)
    .bind(d.carbs_g)
    .bind(d.fat_g)
    .execute(&mut *c)
    .await?;
    Ok(())
}

pub async fn health_last_delivery(c: &mut PgConnection) -> AppResult<Option<DateTime<Utc>>> {
    let a: (Option<DateTime<Utc>>,) = sqlx::query_as(
        "select greatest((select max(updated_at) from health_days), (select max(received_at) from health_workouts), (select max(received_at) from close_signals))",
    )
    .fetch_one(&mut *c)
    .await?;
    Ok(a.0)
}

// ---------- start-up bundle ----------

pub async fn pending_activity(c: &mut PgConnection) -> AppResult<usize> {
    let n: (i64,) = sqlx::query_as("select count(*) from activity where status = 'proposed'").fetch_one(&mut *c).await?;
    Ok(n.0 as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gym_core::split::builtin_by_key;

    #[test]
    fn today_finds_the_scheduled_day_or_the_next_one() {
        let v = PlanVersion {
            id: Uuid::new_v4(),
            previous_id: None,
            created_at: Utc::now(),
            created_by: "you".into(),
            data: PlanData { name: "Strength base".into(), split: builtin_by_key("ppl").unwrap(), block_weeks: Some(6), block_start: NaiveDate::from_ymd_opt(2026, 11, 2), starts_on: None },
        };
        let plan = plan_dto(&v, &[], NaiveDate::from_ymd_opt(2026, 11, 19).unwrap());
        assert_eq!(plan.block_week, Some(3));
        // Thursday 19 Nov 2026 is not a PPL day (Mon, Wed, Fri) -> rest, next is Friday (Legs).
        let t = today(Some(&plan), NaiveDate::from_ymd_opt(2026, 11, 19).unwrap());
        assert!(t.is_rest_day);
        assert_eq!(t.next_day_index, Some(2));
        let mon = today(Some(&plan), NaiveDate::from_ymd_opt(2026, 11, 16).unwrap());
        assert_eq!(mon.day_index, Some(0));
        assert_eq!(mon.next_day_index, Some(1));
    }

    #[test]
    fn local_date_uses_the_phone_offset() {
        let now = DateTime::parse_from_rfc3339("2026-11-19T23:30:00Z").unwrap().with_timezone(&Utc);
        // JavaScript reports UTC+1 as -60.
        assert_eq!(local_date(now, -60), NaiveDate::from_ymd_opt(2026, 11, 20).unwrap());
        assert_eq!(local_date(now, 300), NaiveDate::from_ymd_opt(2026, 11, 19).unwrap());
    }
}
