//! Writes made on someone's behalf.
//!
//! The app (you) and the agent link both change plans, lifts and clips through
//! `submit`. For agents every change is recorded in the activity list with an
//! undo step; "ask me first" turns it into a proposal; a change that would alter
//! a session in progress waits for that session to close. Logged sets are not
//! reachable from here at all.

use chrono::{NaiveDate, Utc};
use gym_core::api::{Activity, ActivityStatus, Clip, Lift, LiftPatch, Prescription};
use gym_core::catalogue::Catalogue;
use gym_core::classify::{classify, may_overwrite};
use gym_core::split::{Split, builtin_by_key};
use gym_core::{LiftTags, TagSource, Unit, Weight};
use serde::{Deserialize, Serialize};
use sqlx::types::Json;
use sqlx::{FromRow, PgConnection, PgPool};
use uuid::Uuid;

use crate::error::{AppError, AppResult, bad};
use crate::store::{self, NewLiftRow, PlanData};

#[derive(Debug, Clone)]
pub enum Actor {
    Owner,
    Agent { id: Uuid, name: String, ask_first: bool },
}

impl Actor {
    pub fn label(&self) -> String {
        match self {
            Actor::Owner => "you".into(),
            Actor::Agent { name, .. } => name.clone(),
        }
    }

    fn tag_source(&self) -> TagSource {
        match self {
            Actor::Owner => TagSource::User,
            Actor::Agent { .. } => TagSource::Agent,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanLift {
    pub name: String,
    pub sets: u32,
    pub reps: u32,
    /// In `unit`, default kg.
    pub weight: f64,
    #[serde(default)]
    pub unit: Option<Unit>,
    #[serde(default)]
    pub rest_seconds: Option<u32>,
    #[serde(default)]
    pub tags: Option<LiftTags>,
    #[serde(default)]
    pub cues: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WritePlan {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub split_key: Option<String>,
    #[serde(default)]
    pub split: Option<Split>,
    #[serde(default)]
    pub block_weeks: Option<u32>,
    /// First day of the plan, if not today.
    #[serde(default)]
    pub starts_on: Option<NaiveDate>,
    #[serde(default)]
    pub lifts: Vec<PlanLift>,
    /// Retire every lift not in `lifts` (they come back with undo).
    #[serde(default)]
    pub retire_others: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    AddLift {
        name: String,
        #[serde(default)]
        prescription: Option<Prescription>,
        #[serde(default)]
        tags: Option<LiftTags>,
        #[serde(default)]
        cues: Option<Vec<String>>,
    },
    UpdateLift {
        lift_id: Uuid,
        patch: LiftPatch,
    },
    AddClip {
        lift_id: Uuid,
        url: String,
        #[serde(default)]
        title: Option<String>,
    },
    RemoveClip {
        clip_id: Uuid,
    },
    SwitchSplit {
        #[serde(default)]
        key: Option<String>,
        #[serde(default)]
        split: Option<Split>,
    },
    WritePlan(WritePlan),
    SetPlanName {
        name: String,
        #[serde(default)]
        block_weeks: Option<u32>,
    },
    /// When the plan begins. None: it has begun.
    SetPlanStart {
        #[serde(default)]
        starts_on: Option<NaiveDate>,
    },
}

impl Op {
    /// Would this change what today's session shows?
    pub fn touches_plan(&self) -> bool {
        match self {
            Op::AddClip { .. } | Op::RemoveClip { .. } | Op::SetPlanName { .. } | Op::SetPlanStart { .. } => false,
            Op::UpdateLift { patch, .. } => patch.prescription.is_some() || patch.tags.is_some() || patch.archived.is_some(),
            _ => true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "undo", rename_all = "snake_case")]
pub enum Undo {
    ArchiveLift { lift_id: Uuid },
    RestoreLift { lift: Box<Lift> },
    DeleteClip { clip_id: Uuid },
    RestoreClip { clip: Box<Clip> },
    RestorePlan { version_id: Option<Uuid> },
    Many { steps: Vec<Undo> },
}

#[derive(Debug, Clone)]
pub struct Applied {
    pub action: &'static str,
    pub summary: String,
    pub detail: Vec<String>,
    pub undo: Undo,
    pub lift_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Outcome {
    pub status: ActivityStatus,
    pub summary: String,
    pub detail: Vec<String>,
    pub activity_id: Option<Uuid>,
    pub lift_id: Option<Uuid>,
}

fn rx(p: &Prescription, unit: Unit) -> String {
    format!("{} × {} at {}", p.sets, p.reps, p.weight.display_with_unit(unit))
}

fn tags_line(t: &LiftTags) -> String {
    format!("{} · {} · {}", t.pattern.label(), t.primary.label(), t.equipment.label())
}

/// Sort a new lift: catalogue, then name rules, then the caller's tags.
fn sort_new(name: &str, given: Option<&LiftTags>, actor: &Actor, cat: &Catalogue) -> (Option<LiftTags>, Option<TagSource>) {
    if let (Actor::Owner, Some(t)) = (actor, given) {
        return (Some(t.clone()), Some(TagSource::User));
    }
    let c = classify(name, cat);
    if c.tags.is_some() {
        return (c.tags, c.source);
    }
    match given {
        Some(t) => (Some(t.clone()), Some(actor.tag_source())),
        None => (None, None),
    }
}

fn resolve_split(key: Option<&str>, split: Option<&Split>) -> AppResult<Split> {
    match (key, split) {
        (_, Some(s)) => {
            let mut s = s.clone();
            // A built-in style on your own days ("ppl@012345") keeps its key, so the app still
            // shows it as push, pull, legs, but only if it is exactly what that style makes.
            let generated = s.key.split_once('@').is_some_and(|(style, code)| {
                let days: Vec<u8> = code.bytes().map(|b| b.wrapping_sub(b'0')).collect();
                gym_core::split::with_days(style, &days).as_ref() == Some(&s)
            });
            if !generated && !s.key.starts_with("custom") && builtin_by_key(&s.key).is_none() {
                s.key = format!("custom:{}", s.key);
            }
            s.validate().map_err(AppError::BadRequest)?;
            Ok(s)
        }
        (Some(k), None) => builtin_by_key(k).ok_or_else(|| bad(format!("No built-in split called {k}. Use ppl, upper_lower, bro or full_body, or send a full split"))),
        (None, None) => Err(bad("Name a split or send one")),
    }
}

pub async fn apply(c: &mut PgConnection, op: &Op, actor: &Actor, cat: &Catalogue, today: NaiveDate) -> AppResult<Applied> {
    let unit = store::settings(c).await?.unit;
    let by = actor.label();
    match op {
        Op::AddLift { name, prescription, tags, cues } => {
            let rxn = prescription.clone().unwrap_or_default();
            let (tags, source) = sort_new(name, tags.as_ref(), actor, cat);
            let id = store::insert_lift(
                c,
                NewLiftRow { name, tags: tags.as_ref(), tag_source: source, prescription: &rxn, cues: cues.as_deref().unwrap_or(&[]), created_by: &by },
            )
            .await?;
            let mut detail = vec![rx(&rxn, unit)];
            detail.push(match &tags {
                Some(t) => format!("Sorted as {}", tags_line(t)),
                None => "Needs a look: it could not be sorted".into(),
            });
            Ok(Applied { action: "add_lift", summary: format!("Added {}", name.trim()), detail, undo: Undo::ArchiveLift { lift_id: id }, lift_id: Some(id) })
        }
        Op::UpdateLift { lift_id, patch } => {
            let before = store::get_lift(c, *lift_id).await?;
            let mut after = before.clone();
            let mut detail = Vec::new();
            if let Some(n) = &patch.name
                && n.trim() != before.name
            {
                after.name = store::check_name(n)?;
                detail.push(format!("Renamed from {}", before.name));
                if after.tags.is_none() {
                    let (t, s) = sort_new(&after.name, None, actor, cat);
                    after.tags = t;
                    after.tag_source = s;
                }
            }
            if let Some(t) = &patch.tags {
                if !may_overwrite(before.tag_source, actor.tag_source()) {
                    return Err(AppError::Forbidden(format!("You set the tags on {} yourself, so agents can't change them", before.name)));
                }
                after.tags = Some(t.clone());
                after.tag_source = Some(actor.tag_source());
                detail.push(format!("Tagged as {}", tags_line(t)));
            }
            if let Some(p) = &patch.prescription {
                store::check_prescription(p)?;
                if *p != before.prescription {
                    detail.push(format!("{} → {}", rx(&before.prescription, unit), rx(p, unit)));
                }
                after.prescription = p.clone();
            }
            if let Some(cues) = &patch.cues {
                after.cues = store::check_cues(cues)?;
                detail.push(format!("{} cues", after.cues.len()));
            }
            if let Some(a) = patch.archived {
                after.archived = a;
                detail.push(if a { "Retired".into() } else { "Brought back".into() });
            }
            store::write_lift(c, &after).await?;
            let verb = if patch.archived == Some(true) { "Retired" } else { "Changed" };
            Ok(Applied { action: "update_lift", summary: format!("{verb} {}", after.name), detail, undo: Undo::RestoreLift { lift: Box::new(before) }, lift_id: Some(*lift_id) })
        }
        Op::AddClip { lift_id, url, title } => {
            let clip = store::insert_clip(c, *lift_id, url, title.as_deref(), &by).await?;
            let lift = store::get_lift(c, *lift_id).await?;
            Ok(Applied {
                action: "add_clip",
                summary: format!("Added a clip to {}", lift.name),
                detail: vec![format!("{} · {}", clip.title, clip.provider.label())],
                undo: Undo::DeleteClip { clip_id: clip.id },
                lift_id: Some(*lift_id),
            })
        }
        Op::RemoveClip { clip_id } => {
            let clip = store::get_clip(c, *clip_id).await?;
            store::delete_clip(c, *clip_id).await?;
            let lift = store::get_lift(c, clip.lift_id).await?;
            Ok(Applied {
                action: "remove_clip",
                summary: format!("Removed a clip from {}", lift.name),
                detail: vec![clip.title.clone()],
                lift_id: Some(clip.lift_id),
                undo: Undo::RestoreClip { clip: Box::new(clip) },
            })
        }
        Op::SwitchSplit { key, split } => {
            let split = resolve_split(key.as_deref(), split.as_ref())?;
            let current = store::current_plan(c).await?;
            let prev = current.as_ref().map(|v| v.id);
            let data = match current {
                Some(v) => PlanData { split: split.clone(), ..v.data },
                None => PlanData { name: "Training plan".into(), split: split.clone(), block_weeks: None, block_start: None, starts_on: None },
            };
            store::push_plan(c, &data, &by).await?;
            Ok(Applied { action: "switch_split", summary: format!("Switched to {}", split.name.to_lowercase()), detail: vec![split.summary.clone()], undo: Undo::RestorePlan { version_id: prev }, lift_id: None })
        }
        Op::SetPlanName { name, block_weeks } => {
            let current = store::current_plan(c).await?.ok_or_else(|| bad("There is no plan yet"))?;
            let prev = current.id;
            let mut data = current.data;
            data.name = name.trim().to_string();
            if block_weeks.is_some() && *block_weeks != data.block_weeks {
                data.block_weeks = *block_weeks;
                data.block_start = Some(today);
            }
            store::push_plan(c, &data, &by).await?;
            Ok(Applied { action: "rename_plan", summary: format!("Renamed the plan {}", data.name), detail: vec![], undo: Undo::RestorePlan { version_id: Some(prev) }, lift_id: None })
        }
        Op::SetPlanStart { starts_on } => {
            let current = store::current_plan(c).await?.ok_or_else(|| bad("There is no plan yet"))?;
            let prev = current.id;
            let mut data = current.data;
            check_start(*starts_on, today)?;
            data.starts_on = *starts_on;
            if data.block_weeks.is_some() {
                data.block_start = Some(starts_on.unwrap_or(today));
            }
            store::push_plan(c, &data, &by).await?;
            let summary = match starts_on {
                Some(d) => format!("The plan starts {}", d.format("%a %-d %b")),
                None => "The plan starts now".to_string(),
            };
            Ok(Applied { action: "plan_start", summary, detail: vec![], undo: Undo::RestorePlan { version_id: Some(prev) }, lift_id: None })
        }
        Op::WritePlan(w) => write_plan(c, w, actor, cat, today, unit).await,
    }
}

/// A start date a year away or more is almost certainly a typo.
fn check_start(starts_on: Option<NaiveDate>, today: NaiveDate) -> AppResult<()> {
    match starts_on {
        Some(d) if d > today + chrono::Duration::days(366) => Err(bad("The start date is more than a year away")),
        Some(d) if d < today - chrono::Duration::days(366) => Err(bad("The start date is more than a year ago")),
        _ => Ok(()),
    }
}

async fn write_plan(c: &mut PgConnection, w: &WritePlan, actor: &Actor, cat: &Catalogue, today: NaiveDate, unit: Unit) -> AppResult<Applied> {
    if w.lifts.len() > 60 {
        return Err(bad("A plan can hold sixty lifts"));
    }
    let by = actor.label();
    let current = store::current_plan(c).await?;
    let prev = current.as_ref().map(|v| v.id);
    let mut undo = Vec::new();
    let mut detail = Vec::new();
    let mut touched: Vec<Uuid> = Vec::new();

    for pl in &w.lifts {
        let lift_unit = pl.unit.unwrap_or(Unit::Kg);
        if !pl.weight.is_finite() || pl.weight < 0.0 {
            return Err(bad(format!("{}: the weight must be a positive number", pl.name)));
        }
        let rxn = Prescription { sets: pl.sets, reps: pl.reps, weight: Weight::from_unit(pl.weight, lift_unit), rest_seconds: pl.rest_seconds };
        store::check_prescription(&rxn).map_err(|e| bad(format!("{}: {}", pl.name, e.message())))?;
        match store::find_lift_by_name(c, &pl.name).await? {
            Some(id) => {
                let before = store::get_lift(c, id).await?;
                let mut after = before.clone();
                after.prescription = rxn.clone();
                if let Some(cues) = &pl.cues {
                    after.cues = store::check_cues(cues)?;
                }
                if let Some(t) = &pl.tags
                    && may_overwrite(before.tag_source, actor.tag_source())
                    && before.tags.as_ref() != Some(t)
                {
                    after.tags = Some(t.clone());
                    after.tag_source = Some(actor.tag_source());
                }
                if after != before {
                    store::write_lift(c, &after).await?;
                    undo.push(Undo::RestoreLift { lift: Box::new(before) });
                }
                touched.push(id);
                detail.push(format!("{} {}", after.name, rx(&rxn, unit)));
            }
            None => {
                let (tags, source) = sort_new(&pl.name, pl.tags.as_ref(), actor, cat);
                let id = store::insert_lift(
                    c,
                    NewLiftRow { name: &pl.name, tags: tags.as_ref(), tag_source: source, prescription: &rxn, cues: pl.cues.as_deref().unwrap_or(&[]), created_by: &by },
                )
                .await?;
                undo.push(Undo::ArchiveLift { lift_id: id });
                touched.push(id);
                let note = if tags.is_none() { " (needs a look)" } else { "" };
                detail.push(format!("{} {} · new{note}", pl.name.trim(), rx(&rxn, unit)));
            }
        }
    }

    if w.retire_others {
        for l in store::list_lifts(c, false).await? {
            if !touched.contains(&l.id) {
                let mut after = l.clone();
                after.archived = true;
                store::write_lift(c, &after).await?;
                detail.push(format!("{} retired", l.name));
                undo.push(Undo::RestoreLift { lift: Box::new(l) });
            }
        }
    }

    let split = if w.split.is_some() || w.split_key.is_some() {
        resolve_split(w.split_key.as_deref(), w.split.as_ref())?
    } else if let Some(v) = &current {
        v.data.split.clone()
    } else {
        builtin_by_key("ppl").expect("built in")
    };
    let mut data = current.map(|v| v.data).unwrap_or(PlanData { name: "Training plan".into(), split: split.clone(), block_weeks: None, block_start: None, starts_on: None });
    data.split = split.clone();
    if let Some(n) = &w.name {
        data.name = n.trim().to_string();
    }
    if w.starts_on.is_some() {
        check_start(w.starts_on, today)?;
        data.starts_on = w.starts_on;
    }
    if w.block_weeks.is_some() {
        data.block_weeks = w.block_weeks;
        data.block_start = Some(w.starts_on.unwrap_or(today));
    }
    store::push_plan(c, &data, &by).await?;
    undo.push(Undo::RestorePlan { version_id: prev });

    let summary = format!("Wrote {}: {}, {} lifts", data.name, split.name.to_lowercase(), w.lifts.len());
    Ok(Applied { action: "write_plan", summary, detail, undo: Undo::Many { steps: undo }, lift_id: None })
}

pub async fn run_undo(c: &mut PgConnection, u: &Undo) -> AppResult<()> {
    match u {
        Undo::ArchiveLift { lift_id } => {
            let mut l = store::get_lift(c, *lift_id).await?;
            l.archived = true;
            store::write_lift(c, &l).await
        }
        Undo::RestoreLift { lift } => store::write_lift(c, lift).await,
        Undo::DeleteClip { clip_id } => store::delete_clip(c, *clip_id).await,
        Undo::RestoreClip { clip } => store::restore_clip(c, clip).await,
        Undo::RestorePlan { version_id } => match version_id {
            Some(id) => store::set_current_plan(c, *id).await,
            None => {
                sqlx::query("delete from settings where key = 'current_plan'").execute(&mut *c).await?;
                Ok(())
            }
        },
        Undo::Many { steps } => {
            for s in steps.iter().rev() {
                Box::pin(run_undo(c, s)).await?;
            }
            Ok(())
        }
    }
}

/// Plain-language description of a change that has not happened yet.
async fn describe(c: &mut PgConnection, op: &Op) -> AppResult<(&'static str, String, Vec<String>)> {
    let unit = store::settings(c).await?.unit;
    Ok(match op {
        Op::AddLift { name, prescription, .. } => ("add_lift", format!("Add {}", name.trim()), vec![rx(&prescription.clone().unwrap_or_default(), unit)]),
        Op::UpdateLift { lift_id, patch } => {
            let l = store::get_lift(c, *lift_id).await?;
            let mut d = Vec::new();
            if let Some(p) = &patch.prescription {
                d.push(format!("{} → {}", rx(&l.prescription, unit), rx(p, unit)));
            }
            if let Some(t) = &patch.tags {
                d.push(format!("Tag as {}", tags_line(t)));
            }
            if patch.archived == Some(true) {
                d.push("Retire it".into());
            }
            ("update_lift", format!("Change {}", l.name), d)
        }
        Op::AddClip { lift_id, url, .. } => {
            let l = store::get_lift(c, *lift_id).await?;
            ("add_clip", format!("Add a clip to {}", l.name), vec![url.clone()])
        }
        Op::RemoveClip { clip_id } => {
            let clip = store::get_clip(c, *clip_id).await?;
            ("remove_clip", "Remove a clip".into(), vec![clip.title])
        }
        Op::SwitchSplit { key, split } => {
            let s = resolve_split(key.as_deref(), split.as_ref())?;
            ("switch_split", format!("Switch to {}", s.name.to_lowercase()), vec![s.summary])
        }
        Op::SetPlanName { name, .. } => ("rename_plan", format!("Rename the plan {name}"), vec![]),
        Op::SetPlanStart { starts_on } => ("plan_start", starts_on.map(|d| format!("Start the plan {}", d.format("%a %-d %b"))).unwrap_or_else(|| "Start the plan now".into()), vec![]),
        Op::WritePlan(w) => (
            "write_plan",
            format!("Write {}", w.name.clone().unwrap_or_else(|| "a new plan".into())),
            w.lifts.iter().map(|l| format!("{} {} × {} at {} {}", l.name, l.sets, l.reps, gym_core::units::format_number(l.weight), l.unit.unwrap_or(Unit::Kg).label())).collect(),
        ),
    })
}

#[allow(clippy::too_many_arguments)]
async fn record(c: &mut PgConnection, actor: &Actor, action: &str, summary: &str, detail: &[String], status: ActivityStatus, op: &Op, undo: Option<&Undo>) -> AppResult<Uuid> {
    let Actor::Agent { id: agent_id, name, .. } = actor else { return Err(AppError::Internal(anyhow::anyhow!("only agent changes are recorded"))) };
    let id = Uuid::new_v4();
    sqlx::query("insert into activity (id, agent_id, agent_name, action, summary, detail, status, op, undo) values ($1, $2, $3, $4, $5, $6, $7, $8, $9)")
        .bind(id)
        .bind(agent_id)
        .bind(name)
        .bind(action)
        .bind(summary)
        .bind(Json(detail))
        .bind(status_str(status))
        .bind(Json(op))
        .bind(undo.map(Json))
        .execute(&mut *c)
        .await?;
    Ok(id)
}

pub fn status_str(s: ActivityStatus) -> &'static str {
    match s {
        ActivityStatus::Applied => "applied",
        ActivityStatus::Proposed => "proposed",
        ActivityStatus::Queued => "queued",
        ActivityStatus::Declined => "declined",
        ActivityStatus::Undone => "undone",
    }
}

fn status_parse(s: &str) -> ActivityStatus {
    match s {
        "proposed" => ActivityStatus::Proposed,
        "queued" => ActivityStatus::Queued,
        "declined" => ActivityStatus::Declined,
        "undone" => ActivityStatus::Undone,
        _ => ActivityStatus::Applied,
    }
}

/// Make a change for you or for an agent. Runs in one transaction.
pub async fn submit(db: &PgPool, actor: &Actor, op: Op, cat: &Catalogue, today: NaiveDate) -> AppResult<Outcome> {
    let mut tx = db.begin().await?;
    let out = submit_in(&mut tx, actor, op, cat, today, false).await?;
    tx.commit().await?;
    Ok(out)
}

/// The most a single change may weigh when stored as JSON. A whole plan of 60 lifts with
/// cues is well under this.
const MAX_OP_BYTES: usize = 64 * 1024;

/// Try a change in a savepoint and roll it back: anything invalid is refused now, before it
/// is stored as a proposal or a queued change.
async fn dry_run(c: &mut PgConnection, actor: &Actor, op: &Op, cat: &Catalogue, today: NaiveDate) -> AppResult<()> {
    let mut sp = sqlx::Acquire::begin(&mut *c).await?;
    let result = apply(&mut sp, op, actor, cat, today).await;
    sp.rollback().await?;
    result.map(|_| ())
}

async fn submit_in(c: &mut PgConnection, actor: &Actor, op: Op, cat: &Catalogue, today: NaiveDate, accepted: bool) -> AppResult<Outcome> {
    if matches!(actor, Actor::Agent { .. }) && serde_json::to_vec(&op).map(|v| v.len()).unwrap_or(usize::MAX) > MAX_OP_BYTES {
        return Err(crate::error::bad("That change is too large. Send smaller pieces"));
    }
    let waits = matches!(actor, Actor::Agent { ask_first: true, .. }) && !accepted
        || matches!(actor, Actor::Agent { .. }) && op.touches_plan() && store::open_session(c).await?.is_some();
    if waits {
        dry_run(c, actor, &op, cat, today).await?;
    }
    if let Actor::Agent { ask_first: true, .. } = actor
        && !accepted
    {
        let (action, summary, detail) = describe(c, &op).await?;
        let id = record(c, actor, action, &summary, &detail, ActivityStatus::Proposed, &op, None).await?;
        return Ok(Outcome { status: ActivityStatus::Proposed, summary, detail, activity_id: Some(id), lift_id: None });
    }
    if matches!(actor, Actor::Agent { .. }) && op.touches_plan() && store::open_session(c).await?.is_some() {
        let (action, summary, detail) = describe(c, &op).await?;
        let id = record(c, actor, action, &summary, &detail, ActivityStatus::Queued, &op, None).await?;
        return Ok(Outcome { status: ActivityStatus::Queued, summary, detail, activity_id: Some(id), lift_id: None });
    }
    let applied = apply(c, &op, actor, cat, today).await?;
    let activity_id = if matches!(actor, Actor::Agent { .. }) {
        Some(record(c, actor, applied.action, &applied.summary, &applied.detail, ActivityStatus::Applied, &op, Some(&applied.undo)).await?)
    } else {
        None
    };
    Ok(Outcome { status: ActivityStatus::Applied, summary: applied.summary, detail: applied.detail, activity_id, lift_id: applied.lift_id })
}

#[derive(FromRow)]
struct ActivityRow {
    id: Uuid,
    agent_id: Option<Uuid>,
    agent_name: String,
    at: chrono::DateTime<Utc>,
    action: String,
    summary: String,
    detail: Json<Vec<String>>,
    status: String,
    op: Json<Op>,
    undo: Option<Json<Undo>>,
}

const ACT_COLS: &str = "id, agent_id, agent_name, at, action, summary, detail, status, op, undo";

async fn get_row(c: &mut PgConnection, id: Uuid) -> AppResult<ActivityRow> {
    let sql = format!("select {ACT_COLS} from activity where id = $1 for update");
    sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(id).fetch_optional(&mut *c).await?.ok_or_else(|| AppError::NotFound("No such change".into()))
}

async fn set_status(c: &mut PgConnection, id: Uuid, s: ActivityStatus, undo: Option<&Undo>, note: Option<&str>) -> AppResult<()> {
    sqlx::query(
        "update activity set status = $2, resolved_at = now(), undo = coalesce($3, undo),
         detail = case when $4::text is null then detail else detail || to_jsonb($4::text) end where id = $1",
    )
    .bind(id)
    .bind(status_str(s))
    .bind(undo.map(Json))
    .bind(note)
    .execute(&mut *c)
    .await?;
    Ok(())
}

pub async fn list_activity(db: &PgPool, limit: i64) -> AppResult<Vec<Activity>> {
    let sql = format!("select {ACT_COLS} from activity order by at desc limit $1");
    let rows: Vec<ActivityRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(limit.clamp(1, 200)).fetch_all(db).await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let status = status_parse(&r.status);
            Activity {
                id: r.id,
                agent_name: r.agent_name,
                at: r.at,
                action: r.action,
                summary: r.summary,
                detail: r.detail.0,
                can_undo: status == ActivityStatus::Applied && r.undo.is_some(),
                status,
            }
        })
        .collect())
}

pub async fn undo_activity(db: &PgPool, id: Uuid) -> AppResult<()> {
    let mut tx = db.begin().await?;
    let row = get_row(&mut tx, id).await?;
    if status_parse(&row.status) != ActivityStatus::Applied {
        return Err(AppError::Conflict("Only an applied change can be undone".into()));
    }
    let Some(Json(u)) = row.undo else { return Err(AppError::Conflict("This change has nothing to undo".into())) };
    run_undo(&mut tx, &u).await?;
    set_status(&mut tx, id, ActivityStatus::Undone, None, None).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn decline_activity(db: &PgPool, id: Uuid) -> AppResult<()> {
    let mut tx = db.begin().await?;
    let row = get_row(&mut tx, id).await?;
    if !matches!(status_parse(&row.status), ActivityStatus::Proposed | ActivityStatus::Queued) {
        return Err(AppError::Conflict("Only a waiting change can be declined".into()));
    }
    set_status(&mut tx, id, ActivityStatus::Declined, None, None).await?;
    tx.commit().await?;
    Ok(())
}

/// The agent behind a recorded change. `ask_first` is off: you already said yes.
fn agent_of(row: &ActivityRow) -> AppResult<Actor> {
    let id = row.agent_id.ok_or_else(|| AppError::Conflict("The agent behind this change is gone".into()))?;
    Ok(Actor::Agent { id, name: row.agent_name.clone(), ask_first: false })
}

/// You accepted a proposal: apply it now, or queue it behind the session in progress.
pub async fn accept_activity(db: &PgPool, id: Uuid, cat: &Catalogue, today: NaiveDate) -> AppResult<ActivityStatus> {
    let mut tx = db.begin().await?;
    let row = get_row(&mut tx, id).await?;
    if status_parse(&row.status) != ActivityStatus::Proposed {
        return Err(AppError::Conflict("This change is not waiting for you".into()));
    }
    let actor = agent_of(&row)?;
    let op = row.op.0.clone();
    let status = if op.touches_plan() && store::open_session(&mut tx).await?.is_some() {
        set_status(&mut tx, id, ActivityStatus::Queued, None, None).await?;
        sqlx::query("update activity set resolved_at = null where id = $1").bind(id).execute(&mut *tx).await?;
        ActivityStatus::Queued
    } else {
        let applied = apply(&mut tx, &op, &actor, cat, today).await?;
        set_status(&mut tx, id, ActivityStatus::Applied, Some(&applied.undo), None).await?;
        ActivityStatus::Applied
    };
    tx.commit().await?;
    Ok(status)
}

/// After a session closes, apply the changes that were waiting for it, oldest first.
/// A change that no longer applies is declined with the reason.
pub async fn flush_queued(db: &PgPool, cat: &Catalogue, today: NaiveDate) -> AppResult<usize> {
    let mut c = db.acquire().await?;
    if store::open_session(&mut c).await?.is_some() {
        return Ok(0);
    }
    let ids: Vec<(Uuid,)> = sqlx::query_as("select id from activity where status = 'queued' order by at").fetch_all(&mut *c).await?;
    drop(c);
    let mut n = 0;
    for (id,) in ids {
        let mut tx = db.begin().await?;
        let row = get_row(&mut tx, id).await?;
        if status_parse(&row.status) != ActivityStatus::Queued {
            continue;
        }
        let actor = agent_of(&row)?;
        let mut sp = sqlx::Acquire::begin(&mut *tx).await?;
        match apply(&mut sp, &row.op.0, &actor, cat, today).await {
            Ok(applied) => {
                sp.commit().await?;
                set_status(&mut tx, id, ActivityStatus::Applied, Some(&applied.undo), None).await?;
                n += 1;
            }
            Err(e) => {
                sp.rollback().await?;
                set_status(&mut tx, id, ActivityStatus::Declined, None, Some(&format!("Could not apply after the session: {}", e.message()))).await?;
            }
        }
        tx.commit().await?;
    }
    Ok(n)
}
