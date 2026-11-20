//! End-to-end checks against a real Postgres, through the real router.
//!
//! Needs `TEST_DATABASE_URL` pointing at a server where the user may create databases,
//! e.g. postgres://gym@127.0.0.1:55432/gym_test. Each test gets its own database.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{Duration, Utc};
use gym_core::api::{Bootstrap, EndSession, SetEntry, StartSession, SyncSets};
use gym_server::admin::{NewAgent, add_agent, new_phone_key};
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
            strava: None,
            strava_verify_token: None,
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
async fn no_agent_tool_returns_strava_or_spotify_data() {
    let t = T::new().await;
    with_session(&t).await;
    sqlx::query("insert into strava_activities (id, data, start_at) values (1, '{\"name\":\"STRAVA_MARKER_RUN\"}', now())").execute(&t.st.db).await.unwrap();
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
    // A run ending closes nothing.
    let (_, v) = t.req("POST", "/ingest/workout-ended", Auth::Ingest(k.clone()), Some(Value::String("kind=Outdoor Run".into()))).await;
    assert_eq!(v["outcome"], "Not a strength workout, so nothing changed");
    assert!(t.bootstrap().await.open_session.is_some());
    // The strength workout ends.
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
async fn strava_list_is_empty_not_an_error_before_any_activity() {
    let t = T::new().await;
    let v = t.ok("GET", "/api/strava/activities", None).await;
    assert_eq!(v, json!([]));
    // Old rows are dropped on read: Strava's terms cap storage at seven days.
    sqlx::query("insert into strava_activities (id, data, start_at, fetched_at) values (9, '{\"name\":\"Old\"}', now() - interval '9 days', now() - interval '9 days')").execute(&t.st.db).await.unwrap();
    let v = t.ok("GET", "/api/strava/activities", None).await;
    assert_eq!(v, json!([]));
    assert_eq!(t.count("select count(*) from strava_activities").await, 0);
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
    // Keys, agent links and Strava or Spotify sign-ins are made on the server only.
    for (method, path) in [("POST", "/api/ingest-key"), ("POST", "/api/agents"), ("POST", "/api/oauth/strava/start"), ("GET", "/api/oauth/spotify/callback"), ("DELETE", "/api/oauth/strava")] {
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
