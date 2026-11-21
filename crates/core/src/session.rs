//! What closes a session (SPEC section 5).
//!
//! The first close signal to arrive closes the session. Anything later only
//! attaches its numbers. A workout record that does not overlap the session in time,
//! or is not a strength workout, changes nothing. The watch ping has no kind (Shortcuts
//! cannot pass it), so it closes the open session at its last set.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

pub const IDLE_MINUTES: i64 = 90;
/// A workout record may start a little before the first logged set.
const LEAD_MINUTES: i64 = 20;
/// …and end a while after the last one (cool-down, walking home with the watch on).
const TAIL_MINUTES: i64 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseReason {
    /// The Apple Watch workout ended (Shortcuts ping or the workout record).
    Watch,
    /// The user tapped End session.
    Tap,
    /// 90 minutes with no new set.
    Idle,
}

impl CloseReason {
    pub fn as_str(self) -> &'static str {
        match self {
            CloseReason::Watch => "watch",
            CloseReason::Tap => "tap",
            CloseReason::Idle => "idle",
        }
    }

    pub fn parse(s: &str) -> Option<CloseReason> {
        Some(match s {
            "watch" => CloseReason::Watch,
            "tap" => CloseReason::Tap,
            "idle" => CloseReason::Idle,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            CloseReason::Watch => "Closed when your watch workout ended",
            CloseReason::Tap => "You ended it",
            CloseReason::Idle => "Closed after 90 minutes without a set",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionWindow {
    pub started_at: DateTime<Utc>,
    pub last_set_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
}

impl SessionWindow {
    pub fn last_activity(&self) -> DateTime<Utc> {
        match self.last_set_at {
            Some(t) if t > self.started_at => t,
            _ => self.started_at,
        }
    }

    /// Does a workout from `start` to `end` belong to this session?
    pub fn overlaps(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> bool {
        let from = self.started_at - Duration::minutes(LEAD_MINUTES);
        let to = self.closed_at.unwrap_or_else(|| self.last_activity()).max(self.last_activity()) + Duration::minutes(TAIL_MINUTES);
        start <= to && end >= from
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Signal {
    /// The Shortcuts "Apple Watch Workout ends" automation. It fires for every workout
    /// type and cannot say which one, so it carries no kind: it only means "a workout ended".
    WorkoutEnded { at: DateTime<Utc> },
    /// A finished workout with its numbers, from the same or the evening automation.
    WorkoutRecord { start: DateTime<Utc>, end: DateTime<Utc>, strength: bool },
    Tap { at: DateTime<Utc> },
    /// Checked lazily on requests and by a timer.
    Idle { now: DateTime<Utc> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Close { at: DateTime<Utc>, reason: CloseReason, attach: bool },
    /// Already closed; keep the numbers but change nothing else.
    AttachOnly,
    Ignore,
}

pub fn decide(s: &SessionWindow, signal: &Signal) -> Decision {
    let open = s.closed_at.is_none();
    match *signal {
        Signal::WorkoutEnded { at } => {
            if !open || at < s.started_at - Duration::minutes(LEAD_MINUTES) {
                return Decision::Ignore;
            }
            // A ping long after the session went quiet is the next workout, not this one.
            if at > s.last_activity() + Duration::minutes(TAIL_MINUTES + IDLE_MINUTES) {
                return Decision::Ignore;
            }
            // The ping may be a run or tennis after the lifting, so its time says nothing
            // about when the lifting stopped. The last logged set does.
            Decision::Close { at: s.last_activity(), reason: CloseReason::Watch, attach: false }
        }
        Signal::WorkoutRecord { start, end, strength } => {
            if !strength || !s.overlaps(start, end) {
                return Decision::Ignore;
            }
            if open {
                Decision::Close { at: end.max(s.last_activity()), reason: CloseReason::Watch, attach: true }
            } else {
                Decision::AttachOnly
            }
        }
        Signal::Tap { at } => {
            if open {
                Decision::Close { at: at.max(s.started_at), reason: CloseReason::Tap, attach: false }
            } else {
                Decision::Ignore
            }
        }
        Signal::Idle { now } => {
            let last = s.last_activity();
            if open && now - last >= Duration::minutes(IDLE_MINUTES) {
                Decision::Close { at: last, reason: CloseReason::Idle, attach: false }
            } else {
                Decision::Ignore
            }
        }
    }
}

/// Apple Health workout type names that count as a gym session.
pub fn is_strength_kind(kind: &str) -> bool {
    let k = kind.to_ascii_lowercase();
    ["strength", "weight", "core training", "functional", "cross training", "crosstraining", "gym"].iter().any(|w| k.contains(w))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 19, h, m, 0).unwrap()
    }

    fn open() -> SessionWindow {
        SessionWindow { started_at: t(18, 0), last_set_at: Some(t(18, 50)), closed_at: None }
    }

    #[test]
    fn watch_ping_closes_an_open_session() {
        let d = decide(&open(), &Signal::WorkoutEnded { at: t(18, 54) });
        assert_eq!(d, Decision::Close { at: open().last_activity(), reason: CloseReason::Watch, attach: false });
    }

    #[test]
    fn a_run_record_closes_nothing() {
        assert_eq!(decide(&open(), &Signal::WorkoutRecord { start: t(18, 0), end: t(18, 54), strength: false }), Decision::Ignore);
        assert!(!is_strength_kind("Outdoor Run"));
        assert!(is_strength_kind("Traditional Strength Training"));
        assert!(is_strength_kind("Functional Strength Training"));
    }

    #[test]
    fn a_run_after_lifting_closes_at_the_last_set_not_the_run_end() {
        // Lifting 18:00–18:50, then a 60-minute run whose end pings at 19:55.
        let d = decide(&open(), &Signal::WorkoutEnded { at: t(19, 55) });
        assert_eq!(d, Decision::Close { at: t(18, 50), reason: CloseReason::Watch, attach: false });
    }

    #[test]
    fn second_signal_is_a_no_op() {
        let mut s = open();
        let first = decide(&s, &Signal::WorkoutEnded { at: t(18, 54) });
        let Decision::Close { at, .. } = first else { panic!("{first:?}") };
        s.closed_at = Some(at);
        assert_eq!(decide(&s, &Signal::WorkoutEnded { at: t(18, 55) }), Decision::Ignore);
        assert_eq!(decide(&s, &Signal::Tap { at: t(19, 0) }), Decision::Ignore);
        assert_eq!(decide(&s, &Signal::Idle { now: t(23, 0) }), Decision::Ignore);
        // The evening automation still attaches the numbers.
        assert_eq!(decide(&s, &Signal::WorkoutRecord { start: t(17, 58), end: t(18, 54), strength: true }), Decision::AttachOnly);
    }

    #[test]
    fn workout_from_another_time_does_not_attach() {
        let mut s = open();
        s.closed_at = Some(t(18, 54));
        assert_eq!(decide(&s, &Signal::WorkoutRecord { start: t(7, 0), end: t(8, 0), strength: true }), Decision::Ignore);
    }

    #[test]
    fn idle_closes_at_the_last_set() {
        assert_eq!(decide(&open(), &Signal::Idle { now: t(20, 19) }), Decision::Ignore);
        assert_eq!(
            decide(&open(), &Signal::Idle { now: t(20, 20) }),
            Decision::Close { at: t(18, 50), reason: CloseReason::Idle, attach: false }
        );
    }

    #[test]
    fn record_closes_when_it_arrives_first() {
        let d = decide(&open(), &Signal::WorkoutRecord { start: t(17, 58), end: t(18, 56), strength: true });
        assert_eq!(d, Decision::Close { at: t(18, 56), reason: CloseReason::Watch, attach: true });
    }

    #[test]
    fn stale_ping_is_ignored() {
        assert_eq!(decide(&open(), &Signal::WorkoutEnded { at: t(23, 30) }), Decision::Ignore);
    }
}
