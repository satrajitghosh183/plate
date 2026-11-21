//! The agent link: MCP over Streamable HTTP, tools only, JSON responses.
//!
//! An agent reaches it at `/mcp/<key>` (the link you give Muse) or at `/mcp` with
//! `Authorization: Bearer <key>`. Requests are stateless: each POST carries one
//! JSON-RPC message.
//!
//! What agents can never do from here: touch logged sets, see Spotify
//! data, or read any key or token. Those simply have no tool.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Duration, Utc};
use gym_core::api::{LiftPatch, Prescription};
use gym_core::split::{Bucket, Split, builtin};
use gym_core::{Equipment, LiftTags, Mechanic, Muscle, Pattern, Unit, Weight};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::progress_for;
use crate::auth::{AgentCaller, agent_from};
use crate::error::AppError;
use crate::ops::{self, Actor, Op, WritePlan};
use crate::state::AppState;
use crate::store::{self, local_date};

/// Only revisions whose text was read; anything else gets the fallback and the client decides.
const VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
const FALLBACK_VERSION: &str = "2025-06-18";

const INSTRUCTIONS: &str = "This is Plate, one person's gym app. You can read their plan, lifts, sessions, progress and daily food and energy totals, \
and you can write plans, lifts, prescriptions, cues and video clips. You cannot change or delete logged sets. \
Weights are in the unit you name (kg by default). Changes made while a session is in progress wait until it ends. \
If the owner set you to ask first, your changes become proposals they accept or decline. Every change you make can be undone by the owner.";

pub async fn post_with_key(State(st): State<AppState>, Path(key): Path<String>, headers: HeaderMap, body: Bytes) -> Response {
    handle(st, headers, Some(key), body).await
}

pub async fn post(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    handle(st, headers, None, body).await
}

/// No server-to-client stream is offered.
pub async fn not_allowed() -> Response {
    (StatusCode::METHOD_NOT_ALLOWED, [(header::ALLOW, "POST")], "").into_response()
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

async fn handle(st: AppState, headers: HeaderMap, path_key: Option<String>, body: Bytes) -> Response {
    // Guard against DNS rebinding from a browser on another site.
    if let Some(origin) = headers.get("origin").and_then(|o| o.to_str().ok())
        && origin != st.cfg.app_origin()
        && origin != st.cfg.public_url.origin().ascii_serialization()
        && !origin.ends_with(".claude.ai")
        && origin != "https://claude.ai"
    {
        return (StatusCode::FORBIDDEN, "Origin not allowed").into_response();
    }
    let agent = match agent_from(&st, &headers, path_key.as_deref()).await {
        Ok(a) => a,
        Err(AppError::TooMany) => return (StatusCode::TOO_MANY_REQUESTS, Json(rpc_error(Value::Null, -32000, "Too many requests. Wait a minute"))).into_response(),
        Err(_) => {
            return (StatusCode::UNAUTHORIZED, [(header::WWW_AUTHENTICATE, "Bearer")], Json(rpc_error(Value::Null, -32001, "This link or key is not valid, or was revoked"))).into_response();
        }
    };
    let msg: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(rpc_error(Value::Null, -32700, "Parse error"))).into_response(),
    };
    // Batches were dropped from the protocol, but answer them anyway.
    if let Value::Array(items) = msg {
        let mut out = Vec::new();
        for item in items {
            if let Some(r) = one(&st, &agent, item).await {
                out.push(r);
            }
        }
        return if out.is_empty() { StatusCode::ACCEPTED.into_response() } else { Json(Value::Array(out)).into_response() };
    }
    match one(&st, &agent, msg).await {
        Some(reply) => Json(reply).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}

async fn one(st: &AppState, agent: &AgentCaller, msg: Value) -> Option<Value> {
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or_default().to_string();
    // Notifications and responses carry no id and get no reply.
    let id = id?;
    if msg.get("jsonrpc").and_then(|v| v.as_str()) != Some("2.0") {
        return Some(rpc_error(id, -32600, "Invalid request"));
    }
    let params = msg.get("params").cloned().unwrap_or(json!({}));
    let result = match method.as_str() {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(|v| v.as_str()).unwrap_or(FALLBACK_VERSION);
            let version = if VERSIONS.contains(&asked) { asked } else { FALLBACK_VERSION };
            Ok(json!({
                "protocolVersion": version,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "plate", "title": "Plate", "version": env!("CARGO_PKG_VERSION") },
                "instructions": INSTRUCTIONS,
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => {
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or_default().to_string();
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            if !tools().iter().any(|t| t["name"] == name.as_str()) {
                return Some(rpc_error(id, -32602, &format!("Unknown tool: {name}")));
            }
            Ok(match call(st, agent, &name, args).await {
                Ok(v) => json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&v).unwrap_or_default() }], "structuredContent": v, "isError": false }),
                Err(e) => json!({ "content": [{ "type": "text", "text": e }], "isError": true }),
            })
        }
        "resources/list" => Ok(json!({ "resources": [] })),
        "prompts/list" => Ok(json!({ "prompts": [] })),
        _ => Err(rpc_error(id.clone(), -32601, &format!("Method not found: {method}"))),
    };
    Some(match result {
        Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
        Err(e) => e,
    })
}

fn enum_values<T: serde::Serialize>(all: &[T]) -> Vec<Value> {
    all.iter().map(|v| serde_json::to_value(v).expect("enum serialises")).collect()
}

fn tags_schema() -> Value {
    json!({
        "type": "object",
        "description": "Only needed when automatic sorting cannot place the lift.",
        "properties": {
            "pattern": { "type": "string", "enum": enum_values(&Pattern::ALL) },
            "primary": { "type": "string", "enum": enum_values(&Muscle::ALL) },
            "secondary": { "type": "array", "items": { "type": "string", "enum": enum_values(&Muscle::ALL) } },
            "equipment": { "type": "string", "enum": enum_values(&Equipment::ALL) },
            "mechanic": { "type": "string", "enum": ["compound", "isolation"] }
        },
        "required": ["pattern", "primary", "equipment", "mechanic"]
    })
}

fn split_schema() -> Value {
    json!({
        "type": "object",
        "description": "A custom split. Each day has a rule: buckets (push, pull, legs, core), muscles, patterns, or everything.",
        "properties": {
            "key": { "type": "string" },
            "name": { "type": "string" },
            "summary": { "type": "string" },
            "days": { "type": "array", "maxItems": 7, "items": { "type": "object", "properties": {
                "name": { "type": "string" },
                "weekday": { "type": "integer", "minimum": 0, "maximum": 6, "description": "0 is Monday" },
                "rule": { "type": "object", "properties": {
                    "kind": { "type": "string", "enum": ["buckets", "muscles", "patterns", "everything"] },
                    "values": { "type": "array", "items": { "type": "string" } }
                }, "required": ["kind"] }
            }, "required": ["name", "weekday", "rule"] } }
        },
        "required": ["key", "name", "days"]
    })
}

fn rx_props() -> Value {
    json!({
        "sets": { "type": "integer", "minimum": 1, "maximum": 20 },
        "reps": { "type": "integer", "minimum": 1, "maximum": 100 },
        "weight": { "type": "number", "minimum": 0, "description": "Working weight in `unit`. 0 for bodyweight." },
        "unit": { "type": "string", "enum": ["kg", "lb"], "default": "kg" },
        "rest_seconds": { "type": "integer", "minimum": 10, "maximum": 900 },
        "cues": { "type": "array", "maxItems": 8, "items": { "type": "string", "maxLength": 160 }, "description": "Short form cues shown under the clip." }
    })
}

fn tool(name: &str, title: &str, description: &str, input: Value, read_only: bool) -> Value {
    json!({
        "name": name,
        "title": title,
        "description": description,
        "inputSchema": input,
        "annotations": { "title": title, "readOnlyHint": read_only, "destructiveHint": false, "idempotentHint": read_only, "openWorldHint": false }
    })
}

pub fn tools() -> Vec<Value> {
    let mut add_props = rx_props();
    add_props["name"] = json!({ "type": "string", "maxLength": 80 });
    add_props["tags"] = tags_schema();
    let mut upd_props = rx_props();
    upd_props["lift_id"] = json!({ "type": "string", "format": "uuid" });
    upd_props["name"] = json!({ "type": "string", "maxLength": 80 });
    upd_props["tags"] = tags_schema();
    upd_props["retire"] = json!({ "type": "boolean", "description": "Take the lift out of the plan. It keeps its history." });
    let mut plan_lift = rx_props();
    plan_lift["name"] = json!({ "type": "string", "maxLength": 80 });
    plan_lift["tags"] = tags_schema();
    vec![
        tool("get_overview", "Overview", "The current plan and its days with prescriptions, today's day, any session in progress, recent sessions, and changes waiting for the owner.", json!({ "type": "object", "properties": {} }), true),
        tool("list_lifts", "List lifts", "Every lift in the pool with its id, tags, prescription, cues and clips. Lifts with no tags need a look and are left out of the plan.", json!({ "type": "object", "properties": { "include_retired": { "type": "boolean" } } }), true),
        tool("get_lift_history", "Lift history", "Estimated one-rep max per session and the recent sets for one lift.", json!({ "type": "object", "properties": { "lift_id": { "type": "string", "format": "uuid" }, "name": { "type": "string" } } }), true),
        tool("list_sessions", "List sessions", "Finished sessions, newest first, with every logged set and the Apple Watch numbers when they arrived.", json!({ "type": "object", "properties": { "limit": { "type": "integer", "minimum": 1, "maximum": 50 }, "before": { "type": "string", "format": "date-time" } } }), true),
        tool("get_health_days", "Food and energy", "Daily totals from Apple Health: energy burned, and energy and macros eaten (logged in Lifesum).", json!({ "type": "object", "properties": { "days": { "type": "integer", "minimum": 1, "maximum": 60 } } }), true),
        tool("list_split_options", "Split options", "The built-in splits and the values tags and custom splits may use.", json!({ "type": "object", "properties": {} }), true),
        tool("add_lift", "Add a lift", "Add one lift to the pool. It is sorted automatically from its name; give tags only if sorting fails.", json!({ "type": "object", "properties": add_props, "required": ["name"] }), false),
        tool("update_lift", "Change a lift", "Change a lift's prescription, cues, name or tags, or retire it. Tags the owner set themselves cannot be changed.", json!({ "type": "object", "properties": upd_props, "required": ["lift_id"] }), false),
        tool(
            "write_plan",
            "Write the plan",
            "Write a whole plan in one step: set prescriptions for the named lifts (adding any that are new), choose the split, and optionally start a training block. Lifts are matched by name.",
            json!({ "type": "object", "properties": {
                "name": { "type": "string", "maxLength": 60, "description": "For example Strength base" },
                "split_key": { "type": "string", "enum": ["ppl", "upper_lower", "bro", "full_body"] },
                "split": split_schema(),
                "block_weeks": { "type": "integer", "minimum": 1, "maximum": 52, "description": "Starts a block of this many weeks today." },
                "retire_others": { "type": "boolean", "description": "Retire every lift not listed. Undo brings them back." },
                "lifts": { "type": "array", "maxItems": 60, "items": { "type": "object", "properties": plan_lift, "required": ["name", "sets", "reps", "weight"] } }
            }, "required": ["lifts"] }),
            false,
        ),
        tool("switch_split", "Switch the split", "Deal the same lifts into a different week. Logged sets are never touched.", json!({ "type": "object", "properties": { "key": { "type": "string", "enum": ["ppl", "upper_lower", "bro", "full_body"] }, "split": split_schema() } }), false),
        tool("add_clip", "Add a clip", "Attach a YouTube video or Instagram reel to a lift. Only the link is stored.", json!({ "type": "object", "properties": { "lift_id": { "type": "string", "format": "uuid" }, "url": { "type": "string", "format": "uri" }, "title": { "type": "string", "maxLength": 80 } }, "required": ["lift_id", "url"] }), false),
        tool("remove_clip", "Remove a clip", "Remove a clip from a lift, for example one that will not play inside the app.", json!({ "type": "object", "properties": { "clip_id": { "type": "string", "format": "uuid" } }, "required": ["clip_id"] }), false),
        tool("list_changes", "Your changes", "Recent changes made by agents and whether each was applied, is waiting, was declined, or was undone.", json!({ "type": "object", "properties": {} }), true),
    ]
}

fn args<T: for<'de> Deserialize<'de>>(v: Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|e| format!("The arguments are not right: {e}"))
}

fn err(e: AppError) -> String {
    e.message()
}

#[derive(Deserialize)]
struct RxArgs {
    #[serde(default)]
    sets: Option<u32>,
    #[serde(default)]
    reps: Option<u32>,
    #[serde(default)]
    weight: Option<f64>,
    #[serde(default)]
    unit: Option<Unit>,
    #[serde(default)]
    rest_seconds: Option<u32>,
    #[serde(default)]
    cues: Option<Vec<String>>,
}

impl RxArgs {
    fn merge(&self, base: &Prescription) -> Result<Option<Prescription>, String> {
        if self.sets.is_none() && self.reps.is_none() && self.weight.is_none() && self.rest_seconds.is_none() {
            return Ok(None);
        }
        let weight = match self.weight {
            Some(w) if !w.is_finite() || w < 0.0 => return Err("The weight must be a positive number".into()),
            Some(w) => Weight::from_unit(w, self.unit.unwrap_or(Unit::Kg)),
            None => base.weight,
        };
        Ok(Some(Prescription { sets: self.sets.unwrap_or(base.sets), reps: self.reps.unwrap_or(base.reps), weight, rest_seconds: self.rest_seconds.or(base.rest_seconds) }))
    }
}

async fn submit(st: &AppState, agent: &AgentCaller, op: Op) -> Result<Value, String> {
    let actor = Actor::Agent { id: agent.id, name: agent.name.clone(), ask_first: agent.ask_first };
    let out = ops::submit(&st.db, &actor, op, &st.catalogue, Utc::now().date_naive()).await.map_err(err)?;
    let note = match out.status {
        gym_core::api::ActivityStatus::Applied => "Applied. The owner can undo it from the app.",
        gym_core::api::ActivityStatus::Proposed => "Sent to the owner to accept or decline. Nothing has changed yet.",
        gym_core::api::ActivityStatus::Queued => "A session is in progress, so this waits and applies when it ends.",
        _ => "",
    };
    Ok(json!({ "status": out.status, "summary": out.summary, "detail": out.detail, "note": note, "lift_id": out.lift_id }))
}

async fn lift_id_by(st: &AppState, id: Option<Uuid>, name: Option<&str>) -> Result<Uuid, String> {
    if let Some(id) = id {
        return Ok(id);
    }
    let name = name.ok_or("Give lift_id or name")?;
    let mut c = st.db.acquire().await.map_err(|e| err(e.into()))?;
    store::find_lift_by_name(&mut c, name).await.map_err(err)?.ok_or_else(|| format!("No lift called {name}"))
}

fn lift_json(l: &gym_core::api::Lift, unit: Unit) -> Value {
    json!({
        "id": l.id, "name": l.name, "needs_look": l.needs_look(), "tags": l.tags, "tag_source": l.tag_source,
        "prescription": { "sets": l.prescription.sets, "reps": l.prescription.reps, "weight": l.prescription.weight.display_value(unit), "unit": unit, "rest_seconds": l.prescription.rest_seconds },
        "cues": l.cues, "retired": l.archived,
        "clips": l.clips.iter().map(|c| json!({ "id": c.id, "title": c.title, "url": c.url, "provider": c.provider, "added_by": c.added_by })).collect::<Vec<_>>(),
    })
}

fn session_json(s: &gym_core::api::Session, names: &dyn Fn(Uuid) -> String, unit: Unit) -> Value {
    json!({
        "id": s.id, "day": s.day_name, "started_at": s.started_at, "closed_at": s.closed_at, "close_reason": s.close_reason,
        "volume": Weight(s.volume()).display_value(unit), "unit": unit,
        "sets": s.sets.iter().map(|x| json!({ "lift": names(x.lift_id), "lift_id": x.lift_id, "weight": x.weight.display_value(unit), "reps": x.reps, "at": x.logged_at })).collect::<Vec<_>>(),
        "watch": s.workout,
    })
}

async fn call(st: &AppState, agent: &AgentCaller, name: &str, a: Value) -> Result<Value, String> {
    let db_err = |e: sqlx::Error| err(e.into());
    match name {
        "get_overview" => {
            let mut c = st.db.acquire().await.map_err(db_err)?;
            let today = local_date(Utc::now(), 0);
            let unit = store::settings(&mut c).await.map_err(err)?.unit;
            let lifts = store::list_lifts(&mut c, false).await.map_err(err)?;
            let name_of = |id: Uuid| lifts.iter().find(|l| l.id == id).map(|l| l.name.clone()).unwrap_or_else(|| "Retired lift".into());
            let plan = store::current_plan(&mut c).await.map_err(err)?.map(|v| store::plan_dto(&v, &lifts, today));
            let t = store::today(plan.as_ref(), today);
            let open = store::open_session(&mut c).await.map_err(err)?;
            let recent = store::recent_sessions(&mut c, 5, None).await.map_err(err)?;
            let pending = store::pending_activity(&mut c).await.map_err(err)?;
            Ok(json!({
                "unit": unit,
                "plan": plan.as_ref().map(|p| json!({
                    "name": p.name, "split": p.split.name, "block_week": p.block_week, "block_weeks": p.block_weeks,
                    "days": p.days.iter().map(|d| json!({
                        "name": d.name, "weekday": d.weekday,
                        "lifts": d.lifts.iter().filter_map(|id| lifts.iter().find(|l| l.id == *id)).map(|l| lift_json(l, unit)).collect::<Vec<_>>()
                    })).collect::<Vec<_>>(),
                    "needs_a_look": p.unplaced.iter().map(|id| name_of(*id)).collect::<Vec<_>>(),
                })),
                "today": plan.as_ref().and_then(|p| t.day_index.map(|i| p.days[i].name.clone())).unwrap_or_else(|| "Rest day".into()),
                "session_in_progress": open.as_ref().map(|s| session_json(s, &name_of, unit)),
                "recent_sessions": recent.iter().map(|s| json!({ "day": s.day_name, "started_at": s.started_at, "sets": s.sets.len(), "volume": Weight(s.volume()).display_value(unit) })).collect::<Vec<_>>(),
                "changes_waiting_for_owner": pending,
            }))
        }
        "list_lifts" => {
            #[derive(Deserialize)]
            struct A {
                #[serde(default)]
                include_retired: bool,
            }
            let a: A = args(a)?;
            let mut c = st.db.acquire().await.map_err(db_err)?;
            let unit = store::settings(&mut c).await.map_err(err)?.unit;
            let lifts = store::list_lifts(&mut c, a.include_retired).await.map_err(err)?;
            Ok(json!({ "lifts": lifts.iter().map(|l| lift_json(l, unit)).collect::<Vec<_>>() }))
        }
        "get_lift_history" => {
            #[derive(Deserialize)]
            struct A {
                lift_id: Option<Uuid>,
                name: Option<String>,
            }
            let a: A = args(a)?;
            let id = lift_id_by(st, a.lift_id, a.name.as_deref()).await?;
            let mut c = st.db.acquire().await.map_err(db_err)?;
            let lift = store::get_lift(&mut c, id).await.map_err(err)?;
            let unit = store::settings(&mut c).await.map_err(err)?.unit;
            let p = progress_for(&mut c, id, unit, 0).await.map_err(err)?;
            Ok(json!({
                "lift": lift.name, "unit": unit,
                "estimated_one_rep_max": p.points.iter().map(|x| json!({ "date": x.date, "e1rm": x.e1rm.display_value(unit), "best_set": x.best_set })).collect::<Vec<_>>(),
                "recent": p.recent.iter().map(|(d, s)| json!({ "date": d, "sets": s })).collect::<Vec<_>>(),
            }))
        }
        "list_sessions" => {
            #[derive(Deserialize)]
            struct A {
                limit: Option<i64>,
                before: Option<DateTime<Utc>>,
            }
            let a: A = args(a)?;
            let mut c = st.db.acquire().await.map_err(db_err)?;
            let unit = store::settings(&mut c).await.map_err(err)?.unit;
            let lifts = store::list_lifts(&mut c, true).await.map_err(err)?;
            let name_of = |id: Uuid| lifts.iter().find(|l| l.id == id).map(|l| l.name.clone()).unwrap_or_default();
            let s = store::recent_sessions(&mut c, a.limit.unwrap_or(10).clamp(1, 50), a.before).await.map_err(err)?;
            Ok(json!({ "sessions": s.iter().map(|x| session_json(x, &name_of, unit)).collect::<Vec<_>>() }))
        }
        "get_health_days" => {
            #[derive(Deserialize)]
            struct A {
                days: Option<i64>,
            }
            let a: A = args(a)?;
            let today = Utc::now().date_naive();
            let mut c = st.db.acquire().await.map_err(db_err)?;
            let days = store::health_days(&mut c, today - Duration::days(a.days.unwrap_or(7).clamp(1, 60) - 1), today).await.map_err(err)?;
            Ok(json!({ "days": days, "note": "Food is logged in Lifesum and reaches Apple Health as day totals; meal times are not reliable." }))
        }
        "list_split_options" => Ok(json!({
            "built_in": builtin().iter().map(|s: &Split| json!({ "key": s.key, "name": s.name, "summary": s.summary, "days": s.days })).collect::<Vec<_>>(),
            "buckets": enum_values(&[Bucket::Push, Bucket::Pull, Bucket::Legs, Bucket::Core]),
            "patterns": enum_values(&Pattern::ALL),
            "muscles": enum_values(&Muscle::ALL),
            "equipment": enum_values(&Equipment::ALL),
            "mechanic": enum_values(&[Mechanic::Compound, Mechanic::Isolation]),
        })),
        "add_lift" => {
            #[derive(Deserialize)]
            struct A {
                name: String,
                #[serde(default)]
                tags: Option<LiftTags>,
                #[serde(flatten)]
                rx: RxArgs,
            }
            let a: A = args(a)?;
            let rx = a.rx.merge(&Prescription::default())?;
            submit(st, agent, Op::AddLift { name: a.name, prescription: rx, tags: a.tags, cues: a.rx.cues }).await
        }
        "update_lift" => {
            #[derive(Deserialize)]
            struct A {
                lift_id: Uuid,
                #[serde(default)]
                name: Option<String>,
                #[serde(default)]
                tags: Option<LiftTags>,
                #[serde(default)]
                retire: Option<bool>,
                #[serde(flatten)]
                rx: RxArgs,
            }
            let a: A = args(a)?;
            let mut c = st.db.acquire().await.map_err(db_err)?;
            let lift = store::get_lift(&mut c, a.lift_id).await.map_err(err)?;
            drop(c);
            let patch = LiftPatch { name: a.name, tags: a.tags, prescription: a.rx.merge(&lift.prescription)?, cues: a.rx.cues, archived: a.retire };
            submit(st, agent, Op::UpdateLift { lift_id: a.lift_id, patch }).await
        }
        "write_plan" => {
            let w: WritePlan = args(a)?;
            submit(st, agent, Op::WritePlan(w)).await
        }
        "switch_split" => {
            #[derive(Deserialize)]
            struct A {
                key: Option<String>,
                split: Option<Split>,
            }
            let a: A = args(a)?;
            submit(st, agent, Op::SwitchSplit { key: a.key, split: a.split }).await
        }
        "add_clip" => {
            #[derive(Deserialize)]
            struct A {
                lift_id: Uuid,
                url: String,
                title: Option<String>,
            }
            let a: A = args(a)?;
            submit(st, agent, Op::AddClip { lift_id: a.lift_id, url: a.url, title: a.title }).await
        }
        "remove_clip" => {
            #[derive(Deserialize)]
            struct A {
                clip_id: Uuid,
            }
            let a: A = args(a)?;
            submit(st, agent, Op::RemoveClip { clip_id: a.clip_id }).await
        }
        "list_changes" => {
            let list = ops::list_activity(&st.db, 30).await.map_err(err)?;
            Ok(json!({ "changes": list.iter().map(|x| json!({ "by": x.agent_name, "at": x.at, "summary": x.summary, "status": x.status })).collect::<Vec<_>>() }))
        }
        _ => Err(format!("Unknown tool {name}")),
    }
}
