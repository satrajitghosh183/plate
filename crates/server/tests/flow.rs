//! End-to-end checks against a real Postgres, through the real router.
//!
//! Needs `TEST_DATABASE_URL` pointing at a server where the user may create databases,
//! e.g. postgres://gym@127.0.0.1:55432/gym_test. Each test gets its own database.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{Duration, Utc};
use gym_core::api::{Bootstrap, EndSession, SetEntry, StartSession, SyncSets};
use gym_server::admin::{NewAgent, add_agent, new_phone_key, new_phone_key_with_overlap};
use gym_core::{Unit, Weight};
use gym_server::config::Config;
use gym_server::crypto::{new_secret, sha256_hex};
use gym_server::state::AppState;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

const ORIGIN: &str = "http://localhost:8080";

struct T {
    /// A session cookie and setup code made fresh for every test: nothing fixed is committed.
    owner: String,
    setup: String,
    app: Router,
    st: AppState,
    admin_url: String,
    db_name: String,
}

impl T {
    async fn new() -> T {
        let base = std::env::var("TEST_DATABASE_URL").expect("set TEST_DATABASE_URL to run these tests");
        let db_name = format!("t_{}", Uuid::new_v4().simple());
        let owner = new_secret("s");
        let setup = new_secret("setup");
        let admin = sqlx::PgPool::connect(&base).await.expect("connect to test server");
        sqlx::query(sqlx::AssertSqlSafe(format!("create database {db_name}"))).execute(&admin).await.unwrap();
        admin.close().await;
        let mut url = url::Url::parse(&base).unwrap();
        url.set_path(&db_name);
        let cfg = Config {
            database_url: url.to_string(),
            port: 0,
            app_url: url::Url::parse(ORIGIN).unwrap(),
            public_url: url::Url::parse(ORIGIN).unwrap(),
            rp_id: "localhost".into(),
            setup_code: Some(setup.clone()),
            token_key: Some(rand_key()),
            cookie_secure: false,
            static_dir: None,
            spotify: None,
        };
        let st = gym_server::build_state(cfg).await.expect("state");
        sqlx::query("insert into auth_sessions (token_hash, expires_at) values ($1, now() + interval '1 day')").bind(sha256_hex(&owner)).execute(&st.db).await.unwrap();
        T { owner, setup, app: gym_server::router(st.clone()), st, admin_url: base, db_name }
    }

    async fn req(&self, method: &str, path: &str, auth: Auth, body: Option<Value>) -> (StatusCode, Value) {
        let mut b = Request::builder().method(method).uri(path);
        match auth {
            Auth::Owner => b = b.header("cookie", format!("gym_session={}", self.owner)).header("origin", ORIGIN),
            Auth::OwnerFrom(o) => b = b.header("cookie", format!("gym_session={}", self.owner)).header("origin", o),
            Auth::Bearer(k) => b = b.header("authorization", format!("Bearer {k}")),
            Auth::Ingest(k) => b = b.header("x-ingest-key", k),
            Auth::None => {}
        }
        let body = match body {
            Some(Value::String(s)) => Body::from(s),
            Some(v) => {
                b = b.header("content-type", "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        let res = self.app.clone().oneshot(b.body(body).unwrap()).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let v = serde_json::from_slice(&bytes).unwrap_or(Value::String(String::from_utf8_lossy(&bytes).into()));
        (status, v)
    }

    /// A POST exactly as the phone sends it: raw bytes, with whatever Content-Type (or none).
    async fn phone(&self, path: &str, key: &str, content_type: Option<&str>, body: &str) -> (StatusCode, Value) {
        let mut b = Request::builder().method("POST").uri(path).header("x-ingest-key", key);
        if let Some(ct) = content_type {
            b = b.header("content-type", ct);
        }
        let res = self.app.clone().oneshot(b.body(Body::from(body.to_string())).unwrap()).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::String(String::from_utf8_lossy(&bytes).into())))
    }

    async fn ok(&self, method: &str, path: &str, body: Option<Value>) -> Value {
        let (s, v) = self.req(method, path, Auth::Owner, body).await;
        assert!(s.is_success(), "{method} {path} -> {s}: {v}");
        v
    }

    async fn bootstrap(&self) -> Bootstrap {
        serde_json::from_value(self.ok("GET", "/api/bootstrap", None).await).unwrap()
    }

    /// Agent links are made on the server (`plate agent add`), never through the app.
    async fn agent(&self, name: &str) -> NewAgent {
        add_agent(&self.st.db, &self.st.cfg, name).await.unwrap()
    }

    /// Phone keys are made on the server (`plate phone-key`), never through the app.
    async fn phone_key(&self) -> String {
        new_phone_key(&self.st.db, &self.st.cfg).await.unwrap().key
    }

    async fn mcp(&self, key: &str, method: &str, params: Value) -> (StatusCode, Value) {
        self.req("POST", &format!("/mcp/{key}"), Auth::None, Some(json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }))).await
    }

    async fn tool(&self, key: &str, name: &str, args: Value) -> Value {
        let (s, v) = self.mcp(key, "tools/call", json!({ "name": name, "arguments": args })).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v["result"].clone()
    }

    async fn count(&self, sql: &str) -> i64 {
        let n: (i64,) = sqlx::query_as(sqlx::AssertSqlSafe(sql.to_string())).fetch_one(&self.st.db).await.unwrap();
        n.0
    }

    async fn done(self) {
        self.st.db.close().await;
        let admin = sqlx::PgPool::connect(&self.admin_url).await.unwrap();
        let _ = sqlx::query(sqlx::AssertSqlSafe(format!("drop database if exists {} with (force)", self.db_name))).execute(&admin).await;
    }
}

fn rand_key() -> [u8; 32] {
    let mut k = [0u8; 32];
    for (i, b) in Uuid::new_v4().as_bytes().iter().chain(Uuid::new_v4().as_bytes()).enumerate() {
        k[i] = *b;
    }
    k
}

enum Auth {
    Owner,
    OwnerFrom(&'static str),
    Bearer(String),
    Ingest(String),
    None,
}

/// Load the starter plan and log two sets of the first lift in a new session.
async fn with_session(t: &T) -> (Bootstrap, Uuid, Vec<SetEntry>) {
    t.ok("POST", "/api/plan/starter", None).await;
    let b = t.bootstrap().await;
    let bench = b.lifts.iter().find(|l| l.name == "Bench press").unwrap().id;
    let session = Uuid::new_v4();
    let start = Utc::now() - Duration::minutes(20);
    let sets: Vec<SetEntry> = (0..2)
        .map(|i| SetEntry { id: Uuid::new_v4(), session_id: session, lift_id: bench, weight: Weight(80_000), reps: 5, unit: Unit::Kg, logged_at: start + Duration::minutes(5 + i * 3) })
        .collect();
    let body = SyncSets { sessions: vec![StartSession { id: session, started_at: start, day_index: Some(0), day_name: "Push".into() }], sets: sets.clone(), deleted: vec![], ended: vec![] };
    t.ok("POST", "/api/sync", Some(serde_json::to_value(&body).unwrap())).await;
    (b, session, sets)
}

#[tokio::test]
async fn starter_plan_deals_the_lifts_by_split() {
    let t = T::new().await;
    let b = t.bootstrap().await;
    assert!(b.plan.is_none() && b.lifts.is_empty(), "a new install starts empty");
    t.ok("POST", "/api/plan/starter", None).await;
    let b = t.bootstrap().await;
    let plan = b.plan.unwrap();
    assert_eq!(plan.days.len(), 3);
    assert!(plan.unplaced.is_empty(), "every starter lift sorts: {:?}", plan.unplaced);
    let names = |i: usize| plan.days[i].lifts.iter().map(|id| b.lifts.iter().find(|l| l.id == *id).unwrap().name.clone()).collect::<Vec<_>>();
    assert!(names(0).contains(&"Bench press".to_string()) && names(0).contains(&"Lateral raise".to_string()), "{:?}", names(0));
    assert!(names(1).contains(&"Pull-up".to_string()) && names(1).contains(&"Face pull".to_string()), "{:?}", names(1));
    assert!(names(2).contains(&"Back squat".to_string()) && names(2).contains(&"Calf raise".to_string()), "{:?}", names(2));
    t.done().await;
}

#[tokio::test]
async fn a_retried_set_is_not_duplicated_and_an_edit_replaces_it() {
    let t = T::new().await;
    let (_, session, mut sets) = with_session(&t).await;
    let again = SyncSets { sessions: vec![], sets: sets.clone(), deleted: vec![], ended: vec![] };
    t.ok("POST", "/api/sync", Some(serde_json::to_value(&again).unwrap())).await;
    t.ok("POST", "/api/sync", Some(serde_json::to_value(&again).unwrap())).await;
    assert_eq!(t.count("select count(*) from sets").await, 2);
    sets[1].reps = 4;
    let edit = SyncSets { sessions: vec![], sets: vec![sets[1].clone()], deleted: vec![], ended: vec![] };
    t.ok("POST", "/api/sync", Some(serde_json::to_value(&edit).unwrap())).await;
    let s = t.ok("GET", &format!("/api/sessions/{session}"), None).await;
    assert_eq!(s["sets"][1]["reps"], 4);
    assert_eq!(t.count("select count(*) from sets").await, 2);
    t.done().await;
}

#[tokio::test]
async fn switching_the_split_leaves_logged_sets_untouched_and_can_be_undone() {
    let t = T::new().await;
    let (_, session, _) = with_session(&t).await;
    let before = t.ok("GET", &format!("/api/sessions/{session}"), None).await;
    let plan = t.ok("POST", "/api/plan/switch", Some(json!({ "key": "bro" }))).await;
    assert_eq!(plan["days"].as_array().unwrap().len(), 5);
    assert_eq!(t.ok("GET", &format!("/api/sessions/{session}"), None).await["sets"], before["sets"]);
    let back = t.ok("POST", "/api/plan/undo", None).await;
    assert_eq!(back["split"]["key"], "ppl");
    assert_eq!(t.count("select count(*) from sets").await, 2);
    t.done().await;
}

#[tokio::test]
async fn agents_cannot_reach_logged_sets() {
    let t = T::new().await;
    let (_, session, sets) = with_session(&t).await;
    let agent = t.agent("Muse").await;
    // The owner routes refuse an agent key outright.
    let body = SyncSets { sessions: vec![], sets: vec![], deleted: vec![sets[0].id], ended: vec![] };
    let (s, _) = t.req("POST", "/api/sync", Auth::Bearer(agent.key.clone()), Some(serde_json::to_value(&body).unwrap())).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _) = t.req("GET", &format!("/api/sessions/{session}"), Auth::Bearer(agent.key.clone()), None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    // And the agent link has no tool for sets.
    let (_, list) = t.mcp(&agent.key, "tools/list", json!({})).await;
    let names: Vec<String> = list["result"]["tools"].as_array().unwrap().iter().map(|x| x["name"].as_str().unwrap().to_string()).collect();
    assert!(names.iter().all(|n| !n.contains("set") || n == "list_sessions" || n == "list_split_options"), "{names:?}");
    let (_, r) = t.mcp(&agent.key, "tools/call", json!({ "name": "delete_set", "arguments": { "id": sets[0].id } })).await;
    assert_eq!(r["error"]["code"], -32602);
    assert_eq!(t.count("select count(*) from sets").await, 2);
    t.done().await;
}

#[tokio::test]
async fn no_agent_tool_returns_spotify_data() {
    let t = T::new().await;
    with_session(&t).await;
    sqlx::query("insert into provider_tokens (provider, ciphertext, nonce, account) values ('spotify', '\\x00', '\\x00', 'SPOTIFY_MARKER_ACCOUNT')").execute(&t.st.db).await.unwrap();
    let agent = t.agent("Claude").await;
    let (_, list) = t.mcp(&agent.key, "tools/list", json!({})).await;
    let text = list.to_string().to_lowercase();
    assert!(!text.contains("strava") && !text.contains("spotify") && !text.contains("music"), "a tool mentions a provider");
    for tool in list["result"]["tools"].as_array().unwrap() {
        if tool["annotations"]["readOnlyHint"] == true {
            let out = t.tool(&agent.key, tool["name"].as_str().unwrap(), json!({ "name": "Bench press" })).await.to_string();
            assert!(!out.contains("STRAVA_MARKER") && !out.contains("SPOTIFY_MARKER"), "{} leaked provider data", tool["name"]);
        }
    }
    t.done().await;
}

#[tokio::test]
async fn the_phone_key_writes_but_cannot_read() {
    let t = T::new().await;
    let k = t.phone_key().await;
    for path in ["/api/bootstrap", "/api/sessions", "/api/export", "/api/health"] {
        let (s, _) = t.req("GET", path, Auth::Bearer(k.clone()), None).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "{path}");
    }
    let (s, _) = t.mcp(&k, "tools/list", json!({})).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, v) = t.req("POST", "/ingest/health", Auth::Ingest(k.clone()), Some(Value::String("date=2026-11-19\ndietary_energy=1420\nactive_energy=610".into()))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, _) = t.req("POST", "/ingest/health", Auth::Ingest(new_secret("phone")), Some(Value::String("date=2026-11-19\ndietary_energy=1".into()))).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    // A new key replaces the old one.
    t.phone_key().await;
    let (s, _) = t.req("POST", "/ingest/workout-ended", Auth::Ingest(k), None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    t.done().await;
}

#[tokio::test]
async fn the_watch_closes_the_session_once_and_numbers_attach_later() {
    let t = T::new().await;
    let (_, session, _) = with_session(&t).await;
    let k = t.phone_key().await;
    // The watch ends a workout (its kind is fixed in the automation, so it is not read).
    let (_, v) = t.req("POST", "/ingest/workout-ended", Auth::Ingest(k.clone()), Some(Value::String("kind=Traditional Strength Training".into()))).await;
    assert_eq!(v["outcome"], "Session closed and saved");
    let first: (Option<chrono::DateTime<Utc>>, Option<String>) = sqlx::query_as("select closed_at, close_reason from workout_sessions where id = $1").bind(session).fetch_one(&t.st.db).await.unwrap();
    assert_eq!(first.1.as_deref(), Some("watch"));
    // The second ping and an End tap change nothing.
    let (_, v) = t.req("POST", "/ingest/workout-ended", Auth::Ingest(k.clone()), None).await;
    assert_eq!(v["outcome"], "No open session to close");
    let tap = SyncSets { sessions: vec![], sets: vec![], deleted: vec![], ended: vec![EndSession { id: session, at: Utc::now() }] };
    t.ok("POST", "/api/sync", Some(serde_json::to_value(&tap).unwrap())).await;
    let second: (Option<chrono::DateTime<Utc>>, Option<String>) = sqlx::query_as("select closed_at, close_reason from workout_sessions where id = $1").bind(session).fetch_one(&t.st.db).await.unwrap();
    assert_eq!(first, second, "a second close signal must be a no-op");
    // The evening automation brings the numbers and attaches them.
    let start = (Utc::now() - Duration::minutes(22)).to_rfc3339();
    let end = Utc::now().to_rfc3339();
    let body = format!("date={}\nworkout=Traditional Strength Training|{start}|{end}|412|128|161", Utc::now().date_naive());
    let (s, v) = t.req("POST", "/ingest/health", Auth::Ingest(k), Some(Value::String(body))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["outcome"].as_str().unwrap().contains("attached"), "{v}");
    let s = t.ok("GET", &format!("/api/sessions/{session}"), None).await;
    assert_eq!(s["workout"]["energy_kcal"], 412.0);
    assert_eq!(s["closed_at"].as_str().map(str::to_string), first.0.map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true)));
    t.done().await;
}

#[tokio::test]
async fn health_days_never_zero_a_stored_number() {
    let t = T::new().await;
    let k = t.phone_key().await;
    let day = Utc::now().date_naive();
    t.req("POST", "/ingest/health", Auth::Ingest(k.clone()), Some(Value::String(format!("date={day}\ndietary_energy=1420\nprotein=96")))).await;
    t.req("POST", "/ingest/health", Auth::Ingest(k.clone()), Some(Value::String(format!("date={day}\ndietary_energy=0\nprotein=0\nactive_energy=610")))).await;
    let h = t.ok("GET", "/api/health?days=1", None).await;
    assert_eq!(h[0]["energy_eaten_kcal"], 1420.0);
    assert_eq!(h[0]["protein_g"], 96.0);
    assert_eq!(h[0]["energy_burned_kcal"], 610.0);
    t.done().await;
}

#[tokio::test]
async fn registration_needs_the_code_and_closes_after_the_first_passkey() {
    let t = T::new().await;
    let (s, _) = t.req("POST", "/api/auth/register/start", Auth::None, Some(json!({ "setup_code": new_secret("wrong") }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, v) = t.req("POST", "/api/auth/register/start", Auth::None, Some(json!({ "setup_code": t.setup }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["options"]["publicKey"]["challenge"].is_string());
    let st: Value = t.req("GET", "/api/auth/status", Auth::None, None).await.1;
    assert_eq!(st["needs_setup"], true);
    // Once a passkey exists, the setup code is not enough.
    sqlx::query("insert into passkeys (id, name, cred) values ($1, 'iPhone', '{}')").bind(Uuid::new_v4()).execute(&t.st.db).await.unwrap();
    let (s, _) = t.req("POST", "/api/auth/register/start", Auth::None, Some(json!({ "setup_code": t.setup }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    t.done().await;
}

#[tokio::test]
async fn writes_from_another_site_are_refused() {
    let t = T::new().await;
    let (s, _) = t.req("POST", "/api/plan/starter", Auth::OwnerFrom("https://evil.example"), None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = t.req("POST", "/api/plan/starter", Auth::None, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    t.done().await;
}

#[tokio::test]
async fn agent_link_speaks_mcp_and_changes_can_be_undone() {
    let t = T::new().await;
    with_session(&t).await;
    // Close the session so changes apply at once.
    let open = t.bootstrap().await.open_session.unwrap().id;
    t.ok("POST", "/api/sync", Some(json!({ "ended": [{ "id": open, "at": Utc::now() }] }))).await;
    let agent = t.agent("Muse").await;
    assert!(agent.link.ends_with(&agent.key) && agent.link.contains("/mcp/"));
    let (s, init) = t.mcp(&agent.key, "initialize", json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test", "version": "1" } })).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    // Notifications get 202 and no body.
    let (s, _) = t.req("POST", &format!("/mcp/{}", agent.key), Auth::None, Some(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))).await;
    assert_eq!(s, StatusCode::ACCEPTED);
    // The header form works too.
    let (s, _) = t.req("POST", "/mcp", Auth::Bearer(agent.key.clone()), Some(json!({ "jsonrpc": "2.0", "id": 2, "method": "ping" }))).await;
    assert_eq!(s, StatusCode::OK);

    let r = t.tool(&agent.key, "add_lift", json!({ "name": "Hip thrust", "sets": 3, "reps": 10, "weight": 100 })).await;
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(r["structuredContent"]["status"], "applied");
    let lifts = t.bootstrap().await.lifts;
    let hip = lifts.iter().find(|l| l.name == "Hip thrust").unwrap();
    assert_eq!(hip.tags.as_ref().unwrap().primary, gym_core::Muscle::Glutes);
    let act = t.ok("GET", "/api/activity", None).await;
    assert_eq!(act[0]["summary"], "Added Hip thrust");
    t.ok("POST", &format!("/api/activity/{}/undo", act[0]["id"].as_str().unwrap()), None).await;
    assert!(t.bootstrap().await.lifts.iter().all(|l| l.name != "Hip thrust"), "undo retires the lift");

    // Tags you set yourself are not the agent's to change.
    let bench = lifts.iter().find(|l| l.name == "Bench press").unwrap().clone();
    t.ok("PATCH", &format!("/api/lifts/{}", bench.id), Some(json!({ "tags": bench.tags }))).await;
    let r = t.tool(&agent.key, "update_lift", json!({ "lift_id": bench.id, "tags": { "pattern": "isolation", "primary": "triceps", "equipment": "cable", "mechanic": "isolation" } })).await;
    assert_eq!(r["isError"], true, "{r}");

    // Revoked keys stop working.
    t.ok("PATCH", &format!("/api/agents/{}", agent.id), Some(json!({ "revoke": true }))).await;
    let (s, _) = t.mcp(&agent.key, "tools/list", json!({})).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    t.done().await;
}

#[tokio::test]
async fn ask_first_makes_a_proposal_and_a_session_in_progress_makes_changes_wait() {
    let t = T::new().await;
    let (_, session, _) = with_session(&t).await;
    let agent = t.agent("Muse").await;
    // A session is open, so a plan change waits.
    let r = t.tool(&agent.key, "switch_split", json!({ "key": "full_body" })).await;
    assert_eq!(r["structuredContent"]["status"], "queued", "{r}");
    assert_eq!(t.bootstrap().await.plan.unwrap().split.key, "ppl");
    // Ending the session applies it.
    t.ok("POST", "/api/sync", Some(json!({ "ended": [{ "id": session, "at": Utc::now() }] }))).await;
    assert_eq!(t.bootstrap().await.plan.unwrap().split.key, "full_body");

    // Ask first: nothing changes until you accept.
    t.ok("PATCH", &format!("/api/agents/{}", agent.id), Some(json!({ "ask_first": true }))).await;
    let r = t.tool(&agent.key, "switch_split", json!({ "key": "upper_lower" })).await;
    assert_eq!(r["structuredContent"]["status"], "proposed");
    let b = t.bootstrap().await;
    assert_eq!(b.plan.unwrap().split.key, "full_body");
    assert_eq!(b.pending_activity, 1);
    let act = t.ok("GET", "/api/activity", None).await;
    let id = act.as_array().unwrap().iter().find(|a| a["status"] == "proposed").unwrap()["id"].as_str().unwrap().to_string();
    t.ok("POST", &format!("/api/activity/{id}/accept"), None).await;
    assert_eq!(t.bootstrap().await.plan.unwrap().split.key, "upper_lower");
    t.done().await;
}

#[tokio::test]
async fn write_plan_builds_a_whole_plan_and_undo_restores_the_old_one() {
    let t = T::new().await;
    t.ok("POST", "/api/plan/starter", None).await;
    let agent = t.agent("Muse").await;
    let r = t
        .tool(
            &agent.key,
            "write_plan",
            json!({
                "name": "Hypertrophy",
                "split_key": "upper_lower",
                "block_weeks": 8,
                "retire_others": true,
                "lifts": [
                    { "name": "Bench press", "sets": 4, "reps": 8, "weight": 72.5 },
                    { "name": "Barbell row", "sets": 4, "reps": 8, "weight": 65 },
                    { "name": "Back squat", "sets": 4, "reps": 8, "weight": 90 },
                    { "name": "Seated cable row", "sets": 3, "reps": 12, "weight": 55 },
                    { "name": "Wibble", "sets": 3, "reps": 12, "weight": 10 }
                ]
            }),
        )
        .await;
    assert_eq!(r["isError"], false, "{r}");
    let b = t.bootstrap().await;
    let plan = b.plan.unwrap();
    assert_eq!((plan.name.as_str(), plan.split.key.as_str(), plan.block_weeks), ("Hypertrophy", "upper_lower", Some(8)));
    assert_eq!(b.lifts.len(), 5);
    assert_eq!(plan.unplaced.len(), 1, "Wibble needs a look");
    let act = t.ok("GET", "/api/activity", None).await;
    t.ok("POST", &format!("/api/activity/{}/undo", act[0]["id"].as_str().unwrap()), None).await;
    let b = t.bootstrap().await;
    assert_eq!(b.plan.unwrap().name, "Strength base");
    assert_eq!(b.lifts.len(), 14);
    assert_eq!(b.lifts.iter().find(|l| l.name == "Bench press").unwrap().prescription.reps, 5);
    t.done().await;
}

#[tokio::test]
async fn clips_store_links_only_and_refuse_bad_ones() {
    let t = T::new().await;
    t.ok("POST", "/api/plan/starter", None).await;
    let bench = t.bootstrap().await.lifts.into_iter().find(|l| l.name == "Bench press").unwrap();
    let l = t.ok("POST", &format!("/api/lifts/{}/clips", bench.id), Some(json!({ "url": "https://youtu.be/rT7DgCr-3pg" }))).await;
    assert_eq!(l["clips"][0]["embed_url"], "https://www.youtube-nocookie.com/embed/rT7DgCr-3pg?playsinline=1&rel=0&modestbranding=1");
    let (s, _) = t.req("POST", &format!("/api/lifts/{}/clips", bench.id), Auth::Owner, Some(json!({ "url": "javascript:alert(1)" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    t.done().await;
}

#[tokio::test]
async fn sets_logged_offline_after_an_idle_close_reopen_the_session() {
    let t = T::new().await;
    let (b, session, _) = with_session(&t).await;
    // The phone went quiet: the server closes the session at its last set after 90 minutes.
    sqlx::query("update workout_sessions set started_at = now() - interval '3 hours', last_set_at = now() - interval '2 hours' where id = $1").bind(session).execute(&t.st.db).await.unwrap();
    sqlx::query("update sets set logged_at = now() - interval '2 hours' where session_id = $1").bind(session).execute(&t.st.db).await.unwrap();
    assert!(t.bootstrap().await.open_session.is_none(), "idle rule closed it");
    // Then the phone comes back with a set it logged offline, after that close.
    let bench = b.lifts.iter().find(|l| l.name == "Bench press").unwrap().id;
    let late = SetEntry { id: Uuid::new_v4(), session_id: session, lift_id: bench, weight: Weight(80_000), reps: 5, unit: Unit::Kg, logged_at: Utc::now() - Duration::minutes(5) };
    t.ok("POST", "/api/sync", Some(json!({ "sets": [late] }))).await;
    let open = t.bootstrap().await.open_session.expect("the session is running again");
    assert_eq!(open.id, session);
    assert_eq!(open.sets.len(), 3);
    t.done().await;
}

#[tokio::test]
async fn nothing_can_be_connected_from_the_app() {
    let t = T::new().await;
    // Keys, agent links and Spotify sign-ins (Strava is gone entirely) are made on the server only.
    for (method, path) in [("POST", "/api/ingest-key"), ("POST", "/api/agents"), ("POST", "/api/oauth/strava/start"), ("GET", "/api/oauth/spotify/callback"), ("DELETE", "/api/oauth/strava"), ("GET", "/api/strava/activities"), ("POST", "/strava/webhook")] {
        let (s, _) = t.req(method, path, Auth::Owner, Some(json!({ "name": "Sneaky" }))).await;
        assert!(s == StatusCode::NOT_FOUND || s == StatusCode::METHOD_NOT_ALLOWED, "{method} {path} answered {s}");
    }
    assert_eq!(t.count("select count(*) from agents").await, 0);
    t.done().await;
}

#[tokio::test]
async fn guessing_the_setup_code_or_phone_key_is_throttled() {
    let t = T::new().await;
    let mut limited = false;
    for _ in 0..40 {
        let (s, _) = t.req("POST", "/api/auth/register/start", Auth::None, Some(json!({ "setup_code": new_secret("guess") }))).await;
        if s == StatusCode::TOO_MANY_REQUESTS {
            limited = true;
            break;
        }
        assert_eq!(s, StatusCode::FORBIDDEN);
    }
    assert!(limited, "40 wrong setup codes in a row were never slowed down");
    let mut limited = false;
    for _ in 0..40 {
        let (s, _) = t.req("POST", "/ingest/workout-ended", Auth::Ingest(new_secret("phone")), None).await;
        if s == StatusCode::TOO_MANY_REQUESTS {
            limited = true;
            break;
        }
    }
    assert!(limited, "40 wrong phone keys in a row were never slowed down");
    t.done().await;
}

async fn raw(t: &T, req: Request<Body>) -> axum::response::Response {
    t.app.clone().oneshot(req).await.unwrap()
}

fn set_cookie(res: &axum::response::Response, name: &str) -> Option<String> {
    res.headers().get_all("set-cookie").iter().filter_map(|v| v.to_str().ok()).find(|v| v.starts_with(&format!("{name}="))).map(str::to_string)
}

#[tokio::test]
async fn without_signing_in_nothing_is_readable() {
    let t = T::new().await;
    for path in ["/api/bootstrap", "/api/agents", "/api/export", "/api/sessions", "/api/health", "/api/activity", "/api/connections"] {
        let (s, _) = t.req("GET", path, Auth::None, None).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "GET {path} answered {s} without a sign-in");
        let (s, _) = t.req("GET", path, Auth::Bearer(new_secret("s")), None).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "GET {path} answered {s} to a made-up token");
    }
    t.done().await;
}

#[tokio::test]
async fn opening_the_app_renews_the_sign_in() {
    let t = T::new().await;
    // A session about to run out.
    sqlx::query("update auth_sessions set expires_at = now() + interval '1 day'").execute(&t.st.db).await.unwrap();
    let res = raw(&t, Request::get("/api/auth/status").header("cookie", format!("gym_session={}", t.owner)).body(Body::empty()).unwrap()).await;
    assert_eq!(res.status(), StatusCode::OK);
    let cookie = set_cookie(&res, "gym_session").expect("the cookie is sent again");
    assert!(cookie.contains("Max-Age=34560000") && cookie.contains("HttpOnly"), "renewed cookie: {cookie}");
    let (days,): (f64,) = sqlx::query_as("select (extract(epoch from (expires_at - now())) / 86400)::float8 from auth_sessions").fetch_one(&t.st.db).await.unwrap();
    assert!(days > 399.0, "session now ends in {days} days");
    // Signed out: no cookie handed out.
    let res = raw(&t, Request::get("/api/auth/status").body(Body::empty()).unwrap()).await;
    assert!(set_cookie(&res, "gym_session").is_none());
    t.done().await;
}

#[tokio::test]
async fn password_sign_in_only_for_your_email_and_password() {
    let t = T::new().await;
    let (s, _) = t.req("POST", "/api/auth/password", Auth::None, Some(json!({ "email": "me@example.com", "password": "anything at all" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "off until set on the server");
    let password = new_secret("pw");
    gym_server::admin::set_owner_email(&t.st.db, "Me@Example.com").await.unwrap();
    gym_server::admin::set_password(&t.st.db, &password).await.unwrap();
    assert!(gym_server::admin::set_password(&t.st.db, "short").await.is_err());
    let (_, v) = t.req("GET", "/api/auth/status", Auth::None, None).await;
    assert_eq!((v["password"].as_bool(), v["google"].as_bool(), v["needs_setup"].as_bool()), (Some(true), Some(false), Some(false)), "{v}");
    let stored: (Value,) = sqlx::query_as("select value from settings where key = 'password_hash'").fetch_one(&t.st.db).await.unwrap();
    assert!(stored.0.as_str().unwrap().starts_with("$argon2id$") && !stored.0.to_string().contains(&password));
    for (email, pw) in [("me@example.com", new_secret("wrong")), ("someone@example.com", password.clone())] {
        let (s, _) = t.req("POST", "/api/auth/password", Auth::None, Some(json!({ "email": email, "password": pw }))).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{email} got in");
    }
    let res = raw(
        &t,
        Request::post("/api/auth/password").header("content-type", "application/json").header("origin", ORIGIN)
            .body(Body::from(json!({ "email": " ME@example.com ", "password": password }).to_string())).unwrap(),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    let token = set_cookie(&res, "gym_session").unwrap().split(';').next().unwrap().trim_start_matches("gym_session=").to_string();
    let res = raw(&t, Request::get("/api/bootstrap").header("cookie", format!("gym_session={token}")).body(Body::empty()).unwrap()).await;
    assert_eq!(res.status(), StatusCode::OK, "the password session opens the app");
    // Another site can't post the form for you.
    let (s, _) = t.req("POST", "/api/auth/password", Auth::None, Some(json!({}))).await;
    assert_ne!(s, StatusCode::OK);
    t.done().await;
}

#[tokio::test]
async fn google_sign_in_goes_to_google_and_refuses_a_forged_return() {
    let t = T::new().await;
    let res = raw(&t, Request::get("/api/auth/google/start").body(Body::empty()).unwrap()).await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN, "off until set on the server");
    gym_server::admin::set_owner_email(&t.st.db, "me@example.com").await.unwrap();
    gym_server::auth::save_google_app(&t.st.db, t.st.cfg.token_key.as_ref().unwrap(), "client-id.apps.googleusercontent.com", &new_secret("gsecret")).await.unwrap();
    let (_, v) = t.req("GET", "/api/auth/status", Auth::None, None).await;
    assert_eq!(v["google"].as_bool(), Some(true), "{v}");
    let stored: (Value,) = sqlx::query_as("select value from settings where key = 'google_app'").fetch_one(&t.st.db).await.unwrap();
    assert!(!stored.0.to_string().contains("gsecret"), "the secret is stored sealed");

    let res = raw(&t, Request::get("/api/auth/google/start").body(Body::empty()).unwrap()).await;
    assert!(res.status().is_redirection(), "{}", res.status());
    let to = url::Url::parse(res.headers()["location"].to_str().unwrap()).unwrap();
    assert_eq!(to.host_str(), Some("accounts.google.com"));
    let q: std::collections::HashMap<_, _> = to.query_pairs().into_owned().collect();
    assert_eq!(q["redirect_uri"], format!("{ORIGIN}/api/auth/google/callback"));
    assert_eq!(q["login_hint"], "me@example.com");
    let ceremony = set_cookie(&res, "gym_google").unwrap().split(';').next().unwrap().to_string();

    // A return whose state doesn't match is refused before Google is ever asked.
    let res = raw(
        &t,
        Request::get(format!("/api/auth/google/callback?code=x&state={}", new_secret("g"))).header("cookie", &ceremony).body(Body::empty()).unwrap(),
    )
    .await;
    assert!(res.headers()["location"].to_str().unwrap().ends_with("/?signin=google-expired"), "{:?}", res.headers()["location"]);
    assert!(set_cookie(&res, "gym_session").is_none(), "no session from a forged return");
    let res = raw(&t, Request::get(format!("/api/auth/google/callback?code=x&state={}", q["state"])).body(Body::empty()).unwrap()).await;
    assert!(set_cookie(&res, "gym_session").is_none(), "no session without the browser's own cookie");
    t.done().await;
}

// ---------- the phone's contract: what iOS Shortcuts actually sends ----------

const PHONE_BODY: &str = "date=2026-11-20\nactive_energy=412.734\ndietary_energy=2,104.5\nprotein=96.2\ncarbs=210\nfat=71.05\n";

type DayRow = (Option<f64>, Option<f64>, Option<f64>, Option<f64>, Option<f64>);

async fn day(t: &T, date: &str) -> Value {
    let rows: Vec<DayRow> =
        sqlx::query_as("select energy_burned_kcal, energy_eaten_kcal, protein_g, carbs_g, fat_g from health_days where date = $1::date").bind(date).fetch_all(&t.st.db).await.unwrap();
    assert!(rows.len() <= 1, "one row per date, got {}", rows.len());
    match rows.first() {
        Some(r) => json!({"burned": r.0, "eaten": r.1, "protein": r.2, "carbs": r.3, "fat": r.4}),
        None => Value::Null,
    }
}

#[tokio::test]
async fn health_takes_the_phones_exact_body_under_any_content_type() {
    let t = T::new().await;
    let k = t.phone_key().await;
    for ct in [Some("text/plain"), Some("application/octet-stream"), None, Some("application/json"), Some("multipart/form-data")] {
        let (s, v) = t.phone("/ingest/health", &k, ct, PHONE_BODY).await;
        assert_eq!(s, StatusCode::OK, "content-type {ct:?}: {v}");
        assert_eq!(v, json!({"ok": true, "outcome": "Saved the day's totals"}), "content-type {ct:?}");
    }
    assert_eq!(day(&t, "2026-11-20").await, json!({"burned": 412.734, "eaten": 2104.5, "protein": 96.2, "carbs": 210.0, "fat": 71.05}));
    t.done().await;
}

#[tokio::test]
async fn health_reads_thousands_separators() {
    let t = T::new().await;
    let k = t.phone_key().await;
    let (s, v) = t.phone("/ingest/health", &k, None, "date=2026-11-20\nactive_energy=1,012.5\ndietary_energy=2,104\nprotein=1,000\ncarbs=96\nfat=7").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let d = day(&t, "2026-11-20").await;
    assert_eq!((d["burned"].as_f64(), d["eaten"].as_f64(), d["protein"].as_f64()), (Some(1012.5), Some(2104.0), Some(1000.0)), "{d}");
    t.done().await;
}

#[tokio::test]
async fn empty_values_from_a_locked_phone_keep_what_is_stored() {
    let t = T::new().await;
    let k = t.phone_key().await;
    // A fresh day with nothing logged: accepted, stored as no reading.
    let (s, v) = t.phone("/ingest/health", &k, Some("text/plain"), "date=2026-11-21\nactive_energy=\ndietary_energy=\nprotein=\ncarbs=\nfat=\n").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v, json!({"ok": true, "outcome": "No numbers in this delivery, so nothing changed"}));
    assert_eq!(day(&t, "2026-11-21").await, json!({"burned": null, "eaten": null, "protein": null, "carbs": null, "fat": null}));
    // Real numbers, then the locked phone's empty and zero values after a workout.
    t.phone("/ingest/health", &k, None, PHONE_BODY).await;
    let (s, _) = t.phone("/ingest/health", &k, None, "date=2026-11-20\nactive_energy=\ndietary_energy=0\nprotein=\ncarbs=0\nfat=").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(day(&t, "2026-11-20").await, json!({"burned": 412.734, "eaten": 2104.5, "protein": 96.2, "carbs": 210.0, "fat": 71.05}));
    t.done().await;
}

#[tokio::test]
async fn repeated_deliveries_for_a_date_keep_one_row_with_the_latest_totals() {
    let t = T::new().await;
    let k = t.phone_key().await;
    for (burned, eaten) in [("120", "450"), ("380.5", "1,240"), ("612", "2,104.5")] {
        let (s, v) = t.phone("/ingest/health", &k, None, &format!("date=2026-11-20\nactive_energy={burned}\ndietary_energy={eaten}\nprotein=50\ncarbs=100\nfat=20")).await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }
    let d = day(&t, "2026-11-20").await;
    assert_eq!((d["burned"].as_f64(), d["eaten"].as_f64()), (Some(612.0), Some(2104.5)), "{d}");
    // A correction downwards (a food entry deleted) also wins: latest, not largest.
    t.phone("/ingest/health", &k, None, "date=2026-11-20\ndietary_energy=1,900").await;
    assert_eq!(day(&t, "2026-11-20").await["eaten"].as_f64(), Some(1900.0));
    t.done().await;
}

#[tokio::test]
async fn health_refuses_a_malformed_body_with_400_and_a_bad_key_with_401() {
    let t = T::new().await;
    let k = t.phone_key().await;
    for body in ["active_energy=412\ndietary_energy=2104\nprotein=96\ncarbs=210\nfat=71", "", "hello", "date=yesterday\nprotein=96", "date=\nprotein=96"] {
        let (s, v) = t.phone("/ingest/health", &k, Some("text/plain"), body).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "body {body:?}: {v}");
    }
    let (s, v) = t.phone("/ingest/health", &k, None, "protein=96").await;
    assert!(v.to_string().contains("date"), "the 400 says what is missing: {v}");
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = t.phone("/ingest/health", "phone_wrong", None, PHONE_BODY).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _) = t.phone("/ingest/health", "", None, PHONE_BODY).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert_eq!(day(&t, "2026-11-20").await, Value::Null, "nothing was saved by a refused call");
    t.done().await;
}

#[tokio::test]
async fn a_tennis_match_ending_is_accepted_and_nothing_trusts_its_kind() {
    let t = T::new().await;
    let k = t.phone_key().await;
    // No session open: the ping (really tennis, saying strength) is accepted, changes nothing.
    let (s, v) = t.phone("/ingest/workout-ended", &k, Some("application/json"), r#"{"kind":"Traditional Strength Training"}"#).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v, json!({"ok": true, "outcome": "No open session to close"}));
    // Lifting, then a run: the run's ping closes the lifting at its last set, not at the ping.
    let (_, session, sets) = with_session(&t).await;
    let (s, v) = t.phone("/ingest/workout-ended", &k, Some("application/json"), r#"{"kind":"Traditional Strength Training"}"#).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["outcome"], "Session closed and saved");
    let (closed,): (Option<chrono::DateTime<Utc>>,) = sqlx::query_as("select closed_at from workout_sessions where id = $1").bind(session).fetch_one(&t.st.db).await.unwrap();
    let last_set = sets.iter().map(|s| s.logged_at).max().unwrap();
    assert_eq!(closed.map(|c| c.timestamp()), Some(last_set.timestamp()), "closed at the last set, not the ping time");
    // Each ping is recorded with a server timestamp and the kind marked unreliable.
    let rows: Vec<(chrono::DateTime<Utc>, Value)> = sqlx::query_as("select received_at, detail from close_signals where kind = 'workout_ended' order by id").fetch_all(&t.st.db).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|(at, d)| d["kind_reliable"] == json!(false) && (Utc::now() - *at).num_minutes() < 5), "{rows:?}");
    t.done().await;
}

#[tokio::test]
async fn a_rotated_key_overlaps_for_a_day_then_stops() {
    let t = T::new().await;
    let old = t.phone_key().await;
    let new = new_phone_key_with_overlap(&t.st.db, &t.st.cfg, 24).await.unwrap().key;
    for k in [&old, &new] {
        let (s, v) = t.phone("/ingest/health", k, None, PHONE_BODY).await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }
    // The overlap ends.
    let ended = (Utc::now() - Duration::minutes(1)).to_rfc3339();
    let n = sqlx::query("update settings set value = jsonb_set(value, '{until}', to_jsonb($1::text)) where key = 'ingest_key_prev'").bind(&ended).execute(&t.st.db).await.unwrap();
    assert_eq!(n.rows_affected(), 1, "the overlap was stored");
    let (s, _) = t.phone("/ingest/health", &old, None, PHONE_BODY).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "the old key stops after the overlap");
    let (s, _) = t.phone("/ingest/health", &new, None, PHONE_BODY).await;
    assert_eq!(s, StatusCode::OK);
    // --now: no overlap at all.
    let newer = new_phone_key_with_overlap(&t.st.db, &t.st.cfg, 0).await.unwrap().key;
    let (s, _) = t.phone("/ingest/health", &new, None, PHONE_BODY).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _) = t.phone("/ingest/health", &newer, None, PHONE_BODY).await;
    assert_eq!(s, StatusCode::OK);
    t.done().await;
}

/// Everything the server logs at every level, while the phone calls it.
#[derive(Clone, Default)]
struct Captured(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn the_phone_key_never_reaches_the_logs() {
    let t = T::new().await;
    let k = t.phone_key().await;
    let buf = Captured::default();
    let w = buf.clone();
    let sub = tracing_subscriber::fmt().with_max_level(tracing::Level::TRACE).with_ansi(false).with_writer(move || w.clone()).finish();
    let _guard = tracing::subscriber::set_default(sub);
    t.phone("/ingest/health", &k, None, PHONE_BODY).await;
    t.phone("/ingest/workout-ended", &k, Some("application/json"), r#"{"kind":"Traditional Strength Training"}"#).await;
    t.phone("/ingest/health", &format!("{k}x"), None, PHONE_BODY).await;
    drop(_guard);
    let logs = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
    assert!(logs.contains("/ingest/health") && logs.contains("workout ended"), "the capture saw the requests:\n{logs}");
    assert!(!logs.contains(&k[6..]), "the key appeared in the logs");
    t.done().await;
}

// ---------- hardening from the security audit ----------

async fn send(t: &T, method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> (StatusCode, Value) {
    let mut b = Request::builder().method(method).uri(path);
    for (k, v) in headers {
        b = b.header(*k, *v);
    }
    let res = t.app.clone().oneshot(b.body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::String(String::from_utf8_lossy(&bytes).into())))
}

#[tokio::test]
async fn forged_addresses_cannot_multiply_sign_in_guesses() {
    let t = T::new().await;
    let mut limited_at = None;
    for i in 0..400u32 {
        let ip = format!("198.51.{}.{}", i / 250, i % 250);
        let (s, _) = send(&t, "POST", "/api/auth/password", &[("x-forwarded-for", &ip), ("content-type", "application/json"), ("origin", ORIGIN)], r#"{"email":"a@b.c","password":"wrong-guess-123"}"#).await;
        if s == StatusCode::TOO_MANY_REQUESTS {
            limited_at = Some(i);
            break;
        }
    }
    let n = limited_at.expect("a new forged address per request must still hit the total limit");
    assert!(n <= 300, "limited only after {n} guesses");
    // Junk in the header shares one bucket instead of minting new ones.
    let t = T::new().await;
    let mut junk_limited = false;
    for i in 0..40 {
        let (s, _) = send(&t, "POST", "/api/auth/password", &[("x-forwarded-for", &format!("not-an-ip-{i}")), ("content-type", "application/json"), ("origin", ORIGIN)], "{}").await;
        junk_limited |= s == StatusCode::TOO_MANY_REQUESTS;
    }
    assert!(junk_limited, "unparseable addresses must share a bucket");
    t.done().await;
}

#[tokio::test]
async fn staying_signed_in_is_never_throttled() {
    let t = T::new().await;
    for i in 0..60 {
        let (s, _) = send(&t, "GET", "/api/auth/status", &[("x-forwarded-for", "203.0.113.9")], "").await;
        assert_eq!(s, StatusCode::OK, "status call {i}");
    }
    t.done().await;
}

#[tokio::test]
async fn agent_keys_never_reach_the_logs() {
    assert_eq!(gym_server::log_path("/mcp/agent_secretsecret"), "/mcp/[key]");
    assert_eq!(gym_server::log_path("/mcp"), "/mcp");
    assert_eq!(gym_server::log_path("/api/bootstrap"), "/api/bootstrap");
    // And at every level, through the real router.
    let t = T::new().await;
    let a = t.agent("Muse").await;
    let buf = Captured::default();
    let w = buf.clone();
    let sub = tracing_subscriber::fmt().with_max_level(tracing::Level::TRACE).with_ansi(false).with_writer(move || w.clone()).finish();
    let guard = tracing::subscriber::set_default(sub);
    t.mcp(&a.key, "tools/list", json!({})).await;
    drop(guard);
    let logs = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
    assert!(logs.contains("/mcp/[key]"), "the request was traced:\n{logs}");
    assert!(!logs.contains(&a.key), "the agent key appeared in the logs");
    t.done().await;
}

#[tokio::test]
async fn a_batch_cannot_smuggle_many_calls() {
    let t = T::new().await;
    let a = t.agent("Muse").await;
    let many: Vec<Value> = (0..11).map(|i| json!({ "jsonrpc": "2.0", "id": i, "method": "tools/list", "params": {} })).collect();
    let (s, v) = t.req("POST", &format!("/mcp/{}", a.key), Auth::None, Some(Value::Array(many))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    let few: Vec<Value> = (0..3).map(|i| json!({ "jsonrpc": "2.0", "id": i, "method": "tools/list", "params": {} })).collect();
    let (s, v) = t.req("POST", &format!("/mcp/{}", a.key), Auth::None, Some(Value::Array(few))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    t.done().await;
}

#[tokio::test]
async fn revoking_an_agent_drops_what_it_left_waiting() {
    let t = T::new().await;
    with_session(&t).await;
    let a = t.agent("Muse").await;
    let out = t.tool(&a.key, "add_lift", json!({ "name": "Zercher squat", "sets": 3, "reps": 6, "weight": 60 })).await;
    let text = out.to_string();
    assert!(text.contains("queued"), "a plan change waits while a session is open: {text}");
    let n = gym_server::admin::revoke_agent(&t.st.db, "Muse").await.unwrap();
    assert_eq!(n, 1);
    assert_eq!(t.count("select count(*) from activity where status in ('queued', 'proposed')").await, 0);
    assert_eq!(t.count("select count(*) from activity where status = 'declined'").await, 1);
    // Ending the session applies nothing from the revoked agent.
    gym_server::ops::flush_queued(&t.st.db, &t.st.catalogue, Utc::now().date_naive()).await.unwrap();
    assert_eq!(t.count("select count(*) from lifts where lower(name) = 'zercher squat'").await, 0);
    t.done().await;
}

#[tokio::test]
async fn invalid_or_huge_changes_are_refused_before_they_wait() {
    let t = T::new().await;
    with_session(&t).await;
    let a = t.agent("Muse").await;
    // Invalid (a bad split) while a session is open: refused now, not stored to fail later.
    let bad = json!({ "split": { "key": "x", "name": "Bad", "summary": "", "days": [] } });
    let (_, v) = t.mcp(&a.key, "tools/call", json!({ "name": "switch_split", "arguments": bad })).await;
    assert!(v.to_string().contains("at least one day"), "{v}");
    // Valid in shape but impossible (the lift exists already): only trying it shows that.
    // Without the dry run it would sit in the queue and fail when the session ends.
    let (_, v) = t.mcp(&a.key, "tools/call", json!({ "name": "add_lift", "arguments": { "name": "Bench press", "sets": 5, "reps": 5, "weight": 80 } })).await;
    assert_eq!(v["result"]["isError"], true, "a duplicate lift must be refused, not queued: {v}");
    // Huge: refused outright.
    let (_, v) = t.mcp(&a.key, "tools/call", json!({ "name": "add_lift", "arguments": { "name": "x".repeat(70_000), "sets": 3, "reps": 5, "weight": 20 } })).await;
    assert!(v.to_string().contains("too large"), "{v}");
    assert_eq!(t.count("select count(*) from activity").await, 0, "nothing was stored");
    t.done().await;
}

#[tokio::test]
async fn the_backup_holds_every_session() {
    let t = T::new().await;
    sqlx::query("insert into workout_sessions (id, started_at, closed_at, close_reason, day_name) select gen_random_uuid(), now() - (g || ' days')::interval, now() - (g || ' days')::interval + interval '1 hour', 'tap', 'Push' from generate_series(1, 230) g")
        .execute(&t.st.db)
        .await
        .unwrap();
    let v = t.ok("GET", "/api/export", None).await;
    assert_eq!(v["sessions"].as_array().map(|a| a.len()), Some(230));
    t.done().await;
}

#[tokio::test]
async fn you_choose_how_many_days_you_train() {
    let t = T::new().await;
    t.ok("POST", "/api/plan/starter", None).await;
    let six = gym_core::split::with_days("ppl", &[0, 1, 2, 3, 4, 5]).unwrap();
    let p = t.ok("POST", "/api/plan/switch", Some(json!({ "split": six }))).await;
    let days = p["days"].as_array().unwrap();
    assert_eq!(days.len(), 6, "{p}");
    assert_eq!(p["split"]["key"], "ppl@012345");
    assert!(days.iter().all(|d| !d["lifts"].as_array().unwrap().is_empty()), "every day gets lifts: {p}");
    // Back to the default three days gives the built-in split.
    let three = gym_core::split::with_days("ppl", &[0, 2, 4]).unwrap();
    let p = t.ok("POST", "/api/plan/switch", Some(json!({ "split": three }))).await;
    assert_eq!(p["split"]["key"], "ppl");
    t.done().await;
}

#[tokio::test]
async fn a_plan_can_start_on_a_later_day() {
    let t = T::new().await;
    t.ok("POST", "/api/plan/starter", None).await;
    let start = Utc::now().date_naive() + Duration::days(10);
    let p = t.ok("POST", "/api/plan/start", Some(json!({ "starts_on": start }))).await;
    assert_eq!(p["starts_on"], json!(start), "{p}");
    let b = t.bootstrap().await;
    assert!(b.today.is_rest_day && b.today.day_index.is_none(), "nothing to train before the start");
    let next = b.today.next_day_index.expect("the first day is named");
    let plan = b.plan.unwrap();
    let swd = chrono::Datelike::weekday(&start).num_days_from_monday() as u8;
    let first = (0..7u8).map(|k| (swd + k) % 7).find(|w| plan.days.iter().any(|d| d.weekday == *w)).unwrap();
    assert_eq!(plan.days[next].weekday, first, "first up is the first training day on or after the start");
    assert_eq!(plan.block_week, Some(1), "the block counts from the start");
    // A typo far in the future is refused; clearing it starts the plan now.
    let (s, _) = t.req("POST", "/api/plan/start", Auth::Owner, Some(json!({ "starts_on": Utc::now().date_naive() + Duration::days(900) }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let p = t.ok("POST", "/api/plan/start", Some(json!({ "starts_on": null }))).await;
    assert!(p["starts_on"].is_null());
    t.done().await;
}
