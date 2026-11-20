//! Request and response bodies shared by the backend and the phone app.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::session::CloseReason;
use crate::split::Split;
use crate::tags::{LiftTags, TagSource};
use crate::units::{Unit, Weight};

// ---------- settings ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub unit: Unit,
    pub rest_seconds: u32,
    pub kcal_goal: u32,
    pub protein_goal_g: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { unit: Unit::Kg, rest_seconds: 120, kcal_goal: 2600, protein_goal_g: 160 }
    }
}

// ---------- lifts ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Prescription {
    pub sets: u32,
    pub reps: u32,
    pub weight: Weight,
    pub rest_seconds: Option<u32>,
}

impl Default for Prescription {
    fn default() -> Self {
        Prescription { sets: 3, reps: 8, weight: Weight(20_000), rest_seconds: None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipProvider {
    Youtube,
    Instagram,
    Other,
}

impl ClipProvider {
    pub fn label(self) -> &'static str {
        match self {
            ClipProvider::Youtube => "YouTube",
            ClipProvider::Instagram => "Reel",
            ClipProvider::Other => "Link",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: Uuid,
    pub lift_id: Uuid,
    pub url: String,
    pub provider: ClipProvider,
    /// The address the in-app player loads, or None when the link cannot embed.
    pub embed_url: Option<String>,
    pub title: String,
    pub added_by: String,
    pub position: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lift {
    pub id: Uuid,
    pub name: String,
    pub tags: Option<LiftTags>,
    pub tag_source: Option<TagSource>,
    pub prescription: Prescription,
    pub cues: Vec<String>,
    pub clips: Vec<Clip>,
    pub archived: bool,
}

impl Lift {
    pub fn needs_look(&self) -> bool {
        self.tags.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewLift {
    pub name: String,
    #[serde(default)]
    pub prescription: Option<Prescription>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiftPatch {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub tags: Option<LiftTags>,
    #[serde(default)]
    pub prescription: Option<Prescription>,
    #[serde(default)]
    pub cues: Option<Vec<String>>,
    #[serde(default)]
    pub archived: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewClip {
    pub url: String,
    #[serde(default)]
    pub title: Option<String>,
}

// ---------- plan ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanDay {
    pub name: String,
    pub weekday: u8,
    pub lifts: Vec<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub version_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
    pub name: String,
    pub block_week: Option<u32>,
    pub block_weeks: Option<u32>,
    pub split: Split,
    pub days: Vec<PlanDay>,
    pub unplaced: Vec<Uuid>,
    /// Is there an older version to go back to?
    pub can_undo: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwitchSplit {
    /// A built-in key ("ppl", "upper_lower", "bro", "full_body") or a full custom split.
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub split: Option<Split>,
}

// ---------- sessions and sets ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetEntry {
    /// Made on the phone so a retry never duplicates the set.
    pub id: Uuid,
    pub session_id: Uuid,
    pub lift_id: Uuid,
    pub weight: Weight,
    pub reps: u32,
    pub unit: Unit,
    pub logged_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StartSession {
    pub id: Uuid,
    pub started_at: DateTime<Utc>,
    pub day_index: Option<u32>,
    pub day_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct WorkoutNumbers {
    pub duration_min: Option<f64>,
    pub energy_kcal: Option<f64>,
    pub avg_heart_rate: Option<f64>,
    pub max_heart_rate: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub id: Uuid,
    pub started_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
    pub close_reason: Option<CloseReason>,
    pub day_name: String,
    pub day_index: Option<u32>,
    pub sets: Vec<SetEntry>,
    pub workout: Option<WorkoutNumbers>,
}

impl Session {
    pub fn volume(&self) -> i64 {
        crate::progress::volume(self.sets.iter().map(|s| (s.weight, s.reps)))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncSets {
    #[serde(default)]
    pub sessions: Vec<StartSession>,
    #[serde(default)]
    pub sets: Vec<SetEntry>,
    #[serde(default)]
    pub deleted: Vec<Uuid>,
    #[serde(default)]
    pub ended: Vec<EndSession>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EndSession {
    pub id: Uuid,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResult {
    pub accepted_sets: Vec<Uuid>,
    pub deleted: Vec<Uuid>,
    pub sessions: Vec<Session>,
}

// ---------- progress ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProgressPoint {
    pub date: NaiveDate,
    pub e1rm: Weight,
    pub best_set: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiftProgress {
    pub lift_id: Uuid,
    pub points: Vec<ProgressPoint>,
    /// Recent sessions, newest first: date and "80 kg × 5, 5, 5, 5".
    pub recent: Vec<(NaiveDate, String)>,
}

// ---------- food and health ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct HealthDay {
    pub date: NaiveDate,
    pub energy_burned_kcal: Option<f64>,
    pub energy_eaten_kcal: Option<f64>,
    pub protein_g: Option<f64>,
    pub carbs_g: Option<f64>,
    pub fat_g: Option<f64>,
}

// ---------- connections ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderStatus {
    /// The backend has an app id and secret for this provider.
    pub configured: bool,
    pub connected: bool,
    pub account: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Connections {
    pub health_last_delivery: Option<DateTime<Utc>>,
    pub health_key_set: bool,
    pub strava: ProviderStatus,
    pub spotify: ProviderStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StravaActivity {
    pub id: i64,
    pub name: String,
    pub sport: String,
    pub start: DateTime<Utc>,
    pub distance_km: f64,
    pub moving_min: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NowPlaying {
    pub playing: bool,
    pub track: Option<String>,
    pub artist: Option<String>,
    pub album_art: Option<String>,
    pub track_url: Option<String>,
    pub device: Option<String>,
    pub no_device: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub image: Option<String>,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MusicCommand {
    /// "play", "pause", "next", "previous", or "playlist".
    pub action: String,
    #[serde(default)]
    pub playlist_id: Option<String>,
}

// ---------- agents ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Agent {
    pub id: Uuid,
    pub name: String,
    pub key_prefix: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub ask_first: bool,
    pub revoked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewAgent {
    pub name: String,
}

/// Returned once, when an agent is created. The key is never shown again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreatedAgent {
    pub agent: Agent,
    pub key: String,
    /// The address to give the agent, with the key in it.
    pub link: String,
    /// The address without the key, for clients that send the key in a header.
    pub endpoint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityStatus {
    Applied,
    /// Waiting for you because the agent is set to ask first.
    Proposed,
    /// Waiting for the session in progress to close.
    Queued,
    Declined,
    Undone,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Activity {
    pub id: Uuid,
    pub agent_name: String,
    pub at: DateTime<Utc>,
    pub action: String,
    pub summary: String,
    pub detail: Vec<String>,
    pub status: ActivityStatus,
    pub can_undo: bool,
}

// ---------- start-up ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Today {
    /// Index into plan.days, None on a rest day with nothing chosen.
    pub day_index: Option<usize>,
    pub is_rest_day: bool,
    pub next_day_index: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bootstrap {
    pub now: DateTime<Utc>,
    pub settings: Settings,
    pub lifts: Vec<Lift>,
    pub plan: Option<Plan>,
    pub today: Today,
    pub open_session: Option<Session>,
    pub recent_sessions: Vec<Session>,
    pub health_week: Vec<HealthDay>,
    pub connections: Connections,
    pub pending_activity: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthStatus {
    pub signed_in: bool,
    /// No way in exists yet (no passkey, password or Google), so the setup code screen is shown.
    pub needs_setup: bool,
    /// Which ways in are switched on. Password and Google are set up on the server only.
    #[serde(default)]
    pub passkey: bool,
    #[serde(default)]
    pub password: bool,
    #[serde(default)]
    pub google: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub error: String,
}
