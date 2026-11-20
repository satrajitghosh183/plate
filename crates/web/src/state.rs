//! Everything the app knows, as signals, plus the phone-side storage that keeps
//! logging working with no signal and while the backend sleeps.
//!
//! - `gym.cache`: the last start-up bundle, so the app opens instantly and offline.
//! - `gym.session`: the session in progress, owned by the phone until it closes.
//! - `gym.outbox`: sessions, sets, deletions and End taps not yet confirmed by the server.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use gym_core::api::{Bootstrap, EndSession, Lift, Plan, Session, SetEntry, StartSession, SyncResult, SyncSets};
use gym_core::{Unit, Weight};
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::bridge;
use crate::net::{self, NetError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Today,
    Plan,
    Progress,
    Fuel,
    Connect,
}

impl Tab {
    pub const ALL: [Tab; 5] = [Tab::Today, Tab::Plan, Tab::Progress, Tab::Fuel, Tab::Connect];
    pub fn index(self) -> usize {
        Tab::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }
    pub fn label(self) -> &'static str {
        match self {
            Tab::Today => "Today",
            Tab::Plan => "Plan",
            Tab::Progress => "Progress",
            Tab::Fuel => "Fuel",
            Tab::Connect => "Connect",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Today,
    Session,
    Logger,
    Rest,
    Summary(Uuid),
}

impl Mode {
    pub fn key(self) -> &'static str {
        match self {
            Mode::Today => "today",
            Mode::Session => "session",
            Mode::Logger => "logger",
            Mode::Rest => "rest",
            Mode::Summary(_) => "summary",
        }
    }
    pub fn in_session(self) -> bool {
        matches!(self, Mode::Session | Mode::Logger | Mode::Rest)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Auth {
    Checking,
    NeedsSetup,
    SignedOut,
    SignedIn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Net {
    Ok,
    Waking,
    Offline,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToastAction {
    UndoPlan,
    RestoreSet(SetEntry),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    pub id: u32,
    pub text: String,
    pub action: Option<(String, ToastAction)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Sheet {
    Clips(Uuid),
    Lift(Uuid),
    Library,
    Music,
    Settings,
    SessionDetail(Uuid),
    AddToSession,
    ChooseDay,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalSession {
    pub id: Uuid,
    pub started_at: DateTime<Utc>,
    pub day_index: Option<u32>,
    pub day_name: String,
    pub lifts: Vec<Uuid>,
    pub sets: Vec<SetEntry>,
    pub current: usize,
    pub rest_until: Option<DateTime<Utc>>,
    pub rest_total: u32,
    pub ended_at: Option<DateTime<Utc>>,
    /// The server confirmed it closed (watch, tap or idle).
    pub closed: bool,
}

impl LocalSession {
    pub fn sets_for(&self, lift: Uuid) -> Vec<&SetEntry> {
        self.sets.iter().filter(|s| s.lift_id == lift).collect()
    }
    pub fn current_lift(&self) -> Option<Uuid> {
        self.lifts.get(self.current).copied()
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Outbox {
    pub sessions: Vec<StartSession>,
    pub sets: Vec<SetEntry>,
    pub deleted: Vec<Uuid>,
    pub ended: Vec<EndSession>,
}

impl Outbox {
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty() && self.sets.is_empty() && self.deleted.is_empty() && self.ended.is_empty()
    }
    pub fn len(&self) -> usize {
        self.sessions.len() + self.sets.len() + self.deleted.len() + self.ended.len()
    }
}

/// The weight and reps on the stepper for each lift, in the display unit.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Drafts(pub HashMap<Uuid, (f64, u32)>);

#[derive(Clone, Copy)]
pub struct Ctx {
    pub auth: RwSignal<Auth>,
    pub data: RwSignal<Option<Bootstrap>>,
    pub tab: RwSignal<Tab>,
    pub mode: RwSignal<Mode>,
    pub session: RwSignal<Option<LocalSession>>,
    pub outbox: RwSignal<Outbox>,
    pub net: RwSignal<Net>,
    pub toast: RwSignal<Option<Toast>>,
    pub sheet: RwSignal<Option<Sheet>>,
    pub chosen_day: RwSignal<Option<usize>>,
    pub drafts: RwSignal<Drafts>,
    pub now: RwSignal<DateTime<Utc>>,
    pub syncing: RwSignal<bool>,
    pub progress_lift: RwSignal<Option<Uuid>>,
}

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

pub fn load<T: for<'de> Deserialize<'de>>(key: &str) -> Option<T> {
    let s = storage()?.get_item(key).ok().flatten()?;
    serde_json::from_str(&s).ok()
}

pub fn save<T: Serialize>(key: &str, v: &T) {
    if let (Some(st), Ok(s)) = (storage(), serde_json::to_string(v)) {
        let _ = st.set_item(key, &s);
    }
}

pub fn forget(key: &str) {
    if let Some(st) = storage() {
        let _ = st.remove_item(key);
    }
}

pub fn ctx() -> Ctx {
    expect_context::<Ctx>()
}

impl Ctx {
    pub fn new() -> Ctx {
        let data: Option<Bootstrap> = load("gym.cache");
        let session: Option<LocalSession> = load("gym.session");
        let mode = match &session {
            Some(s) if !s.closed && s.ended_at.is_none() => {
                if s.rest_until.is_some_and(|t| t > Utc::now()) {
                    Mode::Rest
                } else {
                    Mode::Logger
                }
            }
            _ => Mode::Today,
        };
        Ctx {
            auth: RwSignal::new(if data.is_some() { Auth::SignedIn } else { Auth::Checking }),
            data: RwSignal::new(data),
            tab: RwSignal::new(Tab::Today),
            mode: RwSignal::new(mode),
            session: RwSignal::new(session),
            outbox: RwSignal::new(load("gym.outbox").unwrap_or_default()),
            net: RwSignal::new(if net::online() { Net::Ok } else { Net::Offline }),
            toast: RwSignal::new(None),
            sheet: RwSignal::new(None),
            chosen_day: RwSignal::new(None),
            drafts: RwSignal::new(Drafts::default()),
            now: RwSignal::new(Utc::now()),
            syncing: RwSignal::new(false),
            progress_lift: RwSignal::new(None),
        }
    }

    pub fn unit(&self) -> Unit {
        self.data.with(|d| d.as_ref().map(|d| d.settings.unit).unwrap_or_default())
    }

    pub fn lift(&self, id: Uuid) -> Option<Lift> {
        self.data.with(|d| d.as_ref().and_then(|d| d.lifts.iter().find(|l| l.id == id).cloned()))
    }

    pub fn lift_name(&self, id: Uuid) -> String {
        self.lift(id).map(|l| l.name).unwrap_or_else(|| "Retired lift".into())
    }

    pub fn plan(&self) -> Option<Plan> {
        self.data.with(|d| d.as_ref().and_then(|d| d.plan.clone()))
    }

    pub fn persist_session(&self) {
        match self.session.get_untracked() {
            Some(s) => save("gym.session", &s),
            None => forget("gym.session"),
        }
    }

    pub fn persist_outbox(&self) {
        save("gym.outbox", &self.outbox.get_untracked());
    }

    pub fn say(&self, text: impl Into<String>) {
        self.toast_with(text, None);
    }

    pub fn toast_with(&self, text: impl Into<String>, action: Option<(String, ToastAction)>) {
        let id = self.toast.get_untracked().map(|t| t.id + 1).unwrap_or(1);
        self.toast.set(Some(Toast { id, text: text.into(), action }));
        let toast = self.toast;
        set_timeout(
            move || {
                if toast.get_untracked().map(|t| t.id) == Some(id) {
                    toast.set(None);
                }
            },
            std::time::Duration::from_secs(5),
        );
    }

    pub fn fail(&self, e: &NetError) {
        match e {
            NetError::Unauthorized => self.auth.set(Auth::SignedOut),
            NetError::Unreachable => self.say(if net::online() { "The server is waking up. Try again in a moment" } else { "You're offline. Try again when you have signal" }),
            NetError::Refused(_, m) => self.say(m.clone()),
        }
    }

    // ---------- start-up and refresh ----------

    pub fn start(self) {
        spawn_local(async move {
            match net::get::<gym_core::api::AuthStatus>("/api/auth/status").await {
                Ok(s) if s.signed_in => {
                    self.auth.set(Auth::SignedIn);
                    self.refresh().await;
                }
                Ok(s) => {
                    self.auth.set(if s.needs_setup { Auth::NeedsSetup } else { Auth::SignedOut });
                }
                Err(_) => {
                    // Offline or waking: keep the cached app if there is one, and keep trying.
                    if self.data.get_untracked().is_none() {
                        self.auth.set(Auth::Checking);
                    }
                    self.net.set(if net::online() { Net::Waking } else { Net::Offline });
                    self.wait_for_server().await;
                }
            }
        });
    }

    /// Keep asking until the backend answers (it may take a minute to wake).
    pub async fn wait_for_server(self) {
        for attempt in 0..60u32 {
            gloo_timers::future::TimeoutFuture::new(if attempt < 10 { 2500 } else { 6000 }).await;
            if !net::online() {
                self.net.set(Net::Offline);
                continue;
            }
            if let Ok(s) = net::get::<gym_core::api::AuthStatus>("/api/auth/status").await {
                self.net.set(Net::Ok);
                if s.signed_in {
                    self.auth.set(Auth::SignedIn);
                    self.refresh().await;
                } else {
                    self.auth.set(if s.needs_setup { Auth::NeedsSetup } else { Auth::SignedOut });
                }
                return;
            }
            self.net.set(Net::Waking);
        }
    }

    pub async fn refresh(self) {
        self.flush().await;
        match net::get::<Bootstrap>("/api/bootstrap").await {
            Ok(b) => {
                self.net.set(Net::Ok);
                self.reconcile(&b);
                save("gym.cache", &b);
                self.data.set(Some(b));
            }
            Err(NetError::Unauthorized) => self.auth.set(Auth::SignedOut),
            Err(NetError::Unreachable) => self.net.set(if net::online() { Net::Waking } else { Net::Offline }),
            Err(e) => self.fail(&e),
        }
    }

    pub fn refresh_soon(self) {
        spawn_local(self.refresh());
    }

    /// Bring the phone's session in line with what the server knows.
    fn reconcile(&self, b: &Bootstrap) {
        let local = self.session.get_untracked();
        match (&local, &b.open_session) {
            (None, Some(open)) => {
                // Started elsewhere, or the phone's storage was cleared: adopt it.
                let lifts = lifts_for_day(b.plan.as_ref(), open.day_index.map(|d| d as usize), &open.sets);
                self.session.set(Some(LocalSession {
                    id: open.id,
                    started_at: open.started_at,
                    day_index: open.day_index,
                    day_name: open.day_name.clone(),
                    lifts,
                    sets: open.sets.clone(),
                    current: 0,
                    rest_until: None,
                    rest_total: 0,
                    ended_at: None,
                    closed: false,
                }));
                self.persist_session();
                if self.mode.get_untracked() == Mode::Today {
                    self.mode.set(Mode::Session);
                }
            }
            (Some(l), _) if !l.closed => {
                let pending = self.outbox.with_untracked(|o| o.sessions.iter().any(|s| s.id == l.id));
                let server_closed = b.recent_sessions.iter().find(|s| s.id == l.id).filter(|s| s.closed_at.is_some());
                if let Some(s) = server_closed
                    && !pending
                {
                    // The watch (or the idle rule) closed it.
                    self.session.update(|x| {
                        if let Some(x) = x {
                            x.closed = true;
                            x.rest_until = None;
                            x.sets = s.sets.clone();
                        }
                    });
                    self.persist_session();
                    bridge::set_awake_wanted(false);
                    if self.mode.get_untracked().in_session() {
                        self.mode.set(Mode::Summary(l.id));
                    }
                }
            }
            _ => {}
        }
    }

    // ---------- the outbox ----------

    pub fn queue(&self, f: impl FnOnce(&mut Outbox)) {
        self.outbox.update(f);
        self.persist_outbox();
        let me = *self;
        spawn_local(async move { me.flush().await });
    }

    /// Send what the phone has not yet delivered. Safe to call any time: sets carry
    /// ids made on the phone, so a retry never duplicates them.
    pub async fn flush(self) {
        if self.syncing.get_untracked() || self.outbox.with_untracked(|o| o.is_empty()) {
            return;
        }
        self.syncing.set(true);
        let sent = self.outbox.get_untracked();
        let body = SyncSets { sessions: sent.sessions.clone(), sets: sent.sets.clone(), deleted: sent.deleted.clone(), ended: sent.ended.clone() };
        let res = net::post::<_, SyncResult>("/api/sync", &body).await;
        self.syncing.set(false);
        match res {
            Ok(r) => {
                self.net.set(Net::Ok);
                self.outbox.update(|o| {
                    o.sessions.retain(|s| !sent.sessions.contains(s));
                    o.sets.retain(|s| !sent.sets.contains(s));
                    o.deleted.retain(|d| !sent.deleted.contains(d));
                    o.ended.retain(|e| !sent.ended.contains(e));
                });
                self.persist_outbox();
                self.merge_sessions(r.sessions);
            }
            Err(NetError::Unreachable) => {
                self.net.set(if net::online() { Net::Waking } else { Net::Offline });
                let me = self;
                set_timeout(move || spawn_local(me.flush()), std::time::Duration::from_secs(8));
            }
            Err(NetError::Unauthorized) => self.auth.set(Auth::SignedOut),
            Err(NetError::Refused(code, msg)) if (400..500).contains(&code) => {
                // One bad item must not block every later set, and must not cost the good ones.
                // Send the batch piece by piece and set aside only what is refused.
                self.syncing.set(true);
                let parked = self.flush_one_by_one(&sent).await;
                self.syncing.set(false);
                if parked > 0 {
                    self.say(format!("{parked} change{} could not be saved and {} set aside: {msg}", if parked == 1 { "" } else { "s" }, if parked == 1 { "was" } else { "were" }));
                }
            }
            Err(NetError::Refused(_, msg)) => self.say(format!("A change could not be saved yet: {msg}")),
        }
    }

    /// Send each queued item on its own. Items the server refuses go to `gym.rejected`
    /// (kept on the phone, never silently dropped); the rest are delivered.
    async fn flush_one_by_one(self, sent: &Outbox) -> usize {
        let mut pieces: Vec<SyncSets> = Vec::new();
        for s in &sent.sessions {
            pieces.push(SyncSets { sessions: vec![s.clone()], sets: vec![], deleted: vec![], ended: vec![] });
        }
        for s in &sent.sets {
            pieces.push(SyncSets { sessions: vec![], sets: vec![s.clone()], deleted: vec![], ended: vec![] });
        }
        for d in &sent.deleted {
            pieces.push(SyncSets { sessions: vec![], sets: vec![], deleted: vec![*d], ended: vec![] });
        }
        for e in &sent.ended {
            pieces.push(SyncSets { sessions: vec![], sets: vec![], deleted: vec![], ended: vec![e.clone()] });
        }
        let mut rejected: Vec<(SyncSets, String)> = load("gym.rejected").unwrap_or_default();
        let mut parked = 0;
        for piece in pieces {
            match net::post::<_, SyncResult>("/api/sync", &piece).await {
                Ok(r) => self.merge_sessions(r.sessions),
                Err(NetError::Refused(code, m)) if (400..500).contains(&code) => {
                    parked += 1;
                    rejected.push((piece.clone(), m));
                }
                // Network trouble: leave it in the outbox for the next try.
                Err(_) => continue,
            }
            self.outbox.update(|o| {
                o.sessions.retain(|s| !piece.sessions.contains(s));
                o.sets.retain(|s| !piece.sets.contains(s));
                o.deleted.retain(|d| !piece.deleted.contains(d));
                o.ended.retain(|e| !piece.ended.contains(e));
            });
            self.persist_outbox();
        }
        if parked > 0 {
            save("gym.rejected", &rejected);
        }
        parked
    }

    fn merge_sessions(&self, sessions: Vec<Session>) {
        self.data.update(|d| {
            let Some(d) = d else { return };
            for s in sessions {
                if s.closed_at.is_none() {
                    d.open_session = Some(s);
                } else {
                    if d.open_session.as_ref().is_some_and(|o| o.id == s.id) {
                        d.open_session = None;
                    }
                    match d.recent_sessions.iter_mut().find(|x| x.id == s.id) {
                        Some(x) => *x = s,
                        None => d.recent_sessions.insert(0, s),
                    }
                }
            }
            d.recent_sessions.sort_by_key(|s| std::cmp::Reverse(s.started_at));
        });
        if let Some(d) = self.data.get_untracked() {
            save("gym.cache", &d);
        }
    }

    // ---------- training ----------

    pub fn start_session(&self, day_index: Option<usize>) {
        let plan = self.plan();
        let (name, lifts) = match (plan.as_ref(), day_index) {
            (Some(p), Some(i)) if i < p.days.len() => (p.days[i].name.clone(), p.days[i].lifts.clone()),
            _ => ("Training".to_string(), Vec::new()),
        };
        let s = LocalSession {
            id: Uuid::new_v4(),
            started_at: Utc::now(),
            day_index: day_index.map(|i| i as u32),
            day_name: name.clone(),
            lifts,
            sets: Vec::new(),
            current: 0,
            rest_until: None,
            rest_total: 0,
            ended_at: None,
            closed: false,
        };
        let start = StartSession { id: s.id, started_at: s.started_at, day_index: s.day_index, day_name: name };
        self.session.set(Some(s));
        self.persist_session();
        bridge::unlock_audio();
        bridge::set_awake_wanted(true);
        self.queue(|o| o.sessions.push(start));
        self.mode.set(Mode::Logger);
    }

    pub fn log_set(&self, lift_id: Uuid, value: f64, reps: u32) {
        let unit = self.unit();
        let Some(s) = self.session.get_untracked() else { return };
        let entry = SetEntry { id: Uuid::new_v4(), session_id: s.id, lift_id, weight: Weight::from_unit(value, unit), reps, unit, logged_at: Utc::now() };
        let rest = self.lift(lift_id).and_then(|l| l.prescription.rest_seconds).unwrap_or_else(|| self.data.with_untracked(|d| d.as_ref().map(|d| d.settings.rest_seconds).unwrap_or(120)));
        let target = self.lift(lift_id).map(|l| l.prescription.sets).unwrap_or(3) as usize;
        let mut finished_lift = false;
        let mut finished_all = false;
        self.session.update(|x| {
            let Some(x) = x else { return };
            x.sets.push(entry.clone());
            if !x.lifts.contains(&lift_id) {
                x.lifts.push(lift_id);
            }
            finished_lift = x.sets_for(lift_id).len() >= target;
            if finished_lift {
                // Move on to the first lift that still has sets to do.
                let next = (0..x.lifts.len()).map(|k| (x.current + 1 + k) % x.lifts.len()).find(|&i| {
                    let id = x.lifts[i];
                    let want = self.lift(id).map(|l| l.prescription.sets).unwrap_or(3) as usize;
                    x.sets_for(id).len() < want
                });
                match next {
                    Some(i) => x.current = i,
                    None => finished_all = true,
                }
            }
            x.rest_until = if finished_all { None } else { Some(Utc::now() + chrono::Duration::seconds(rest as i64)) };
            x.rest_total = rest;
        });
        self.persist_session();
        self.queue(|o| o.sets.push(entry));
        self.mode.set(if finished_all { Mode::Session } else { Mode::Rest });
        if finished_all {
            self.say("Every set is done. End the session when you're ready, or let your watch do it");
        }
    }

    pub fn end_session(&self) {
        let Some(s) = self.session.get_untracked() else { return };
        let at = Utc::now();
        self.session.update(|x| {
            if let Some(x) = x {
                x.ended_at = Some(at);
                x.rest_until = None;
            }
        });
        self.persist_session();
        bridge::set_awake_wanted(false);
        self.queue(|o| o.ended.push(EndSession { id: s.id, at }));
        self.mode.set(Mode::Summary(s.id));
    }

    pub fn finish_summary(&self) {
        self.session.set(None);
        self.persist_session();
        self.mode.set(Mode::Today);
        self.chosen_day.set(None);
        self.refresh_soon();
    }

    pub fn delete_set(&self, set: &SetEntry) {
        let id = set.id;
        self.session.update(|x| {
            if let Some(x) = x {
                x.sets.retain(|s| s.id != id);
            }
        });
        self.persist_session();
        self.data.update(|d| {
            if let Some(d) = d {
                for s in d.recent_sessions.iter_mut().chain(d.open_session.iter_mut()) {
                    s.sets.retain(|x| x.id != id);
                }
            }
        });
        let unsent = self.outbox.with_untracked(|o| o.sets.iter().any(|s| s.id == id));
        self.queue(|o| {
            o.sets.retain(|s| s.id != id);
            if !unsent {
                o.deleted.push(id);
            }
        });
        self.toast_with("Set deleted", Some(("Undo".into(), ToastAction::RestoreSet(set.clone()))));
    }

    pub fn restore_set(&self, set: SetEntry) {
        let id = set.id;
        self.session.update(|x| {
            if let Some(x) = x
                && x.id == set.session_id
            {
                x.sets.push(set.clone());
                x.sets.sort_by_key(|s| s.logged_at);
            }
        });
        self.persist_session();
        self.data.update(|d| {
            if let Some(d) = d {
                for s in d.recent_sessions.iter_mut().filter(|s| s.id == set.session_id) {
                    s.sets.push(set.clone());
                    s.sets.sort_by_key(|x| x.logged_at);
                }
            }
        });
        self.queue(|o| {
            o.deleted.retain(|d| *d != id);
            o.sets.push(set);
        });
    }

    /// The session to show on the summary: the phone's copy, or the server's.
    pub fn session_view(&self, id: Uuid) -> Option<Session> {
        let local = self.session.get();
        let server = self.data.with(|d| d.as_ref().and_then(|d| d.recent_sessions.iter().chain(d.open_session.iter()).find(|s| s.id == id).cloned()));
        match (local, server) {
            (_, Some(s)) if s.closed_at.is_some() => Some(s),
            (Some(l), s) if l.id == id => Some(Session {
                id: l.id,
                started_at: l.started_at,
                closed_at: s.as_ref().and_then(|s| s.closed_at).or(l.ended_at),
                close_reason: s.as_ref().and_then(|s| s.close_reason),
                day_name: l.day_name.clone(),
                day_index: l.day_index,
                sets: l.sets.clone(),
                workout: s.and_then(|s| s.workout),
            }),
            (_, s) => s,
        }
    }
}

/// The lifts a session trains: the plan day's lifts, plus any lift already logged.
pub fn lifts_for_day(plan: Option<&Plan>, day: Option<usize>, sets: &[SetEntry]) -> Vec<Uuid> {
    let mut lifts: Vec<Uuid> = plan.and_then(|p| day.and_then(|i| p.days.get(i))).map(|d| d.lifts.clone()).unwrap_or_default();
    for s in sets {
        if !lifts.contains(&s.lift_id) {
            lifts.push(s.lift_id);
        }
    }
    lifts
}
