//! Splits are data. A split is an ordered list of days, each with a rule for which
//! lifts belong. Dealing puts every tagged lift from the pool onto a day.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::tags::{LiftTags, Mechanic, Muscle, Pattern};

/// The coarse family a lift belongs to, used by push/pull/legs style rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Bucket {
    Push,
    Pull,
    Legs,
    Core,
}

impl Bucket {
    pub fn label(self) -> &'static str {
        match self {
            Bucket::Push => "Push",
            Bucket::Pull => "Pull",
            Bucket::Legs => "Legs",
            Bucket::Core => "Core",
        }
    }
}

pub fn bucket(tags: &LiftTags) -> Bucket {
    use Muscle as M;
    match tags.pattern {
        Pattern::HorizontalPush | Pattern::VerticalPush => Bucket::Push,
        Pattern::HorizontalPull | Pattern::VerticalPull | Pattern::Carry => Bucket::Pull,
        Pattern::Squat | Pattern::Hinge | Pattern::Lunge => Bucket::Legs,
        Pattern::Core => Bucket::Core,
        Pattern::Isolation => match tags.primary {
            M::Chest | M::Shoulders | M::Triceps => Bucket::Push,
            M::Lats | M::MiddleBack | M::Traps | M::Biceps | M::Forearms | M::Neck => Bucket::Pull,
            M::Quadriceps | M::Hamstrings | M::Glutes | M::Calves | M::Adductors | M::Abductors | M::LowerBack => Bucket::Legs,
            M::Abdominals => Bucket::Core,
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "values", rename_all = "snake_case")]
pub enum DayRule {
    /// Lifts whose bucket is in the list.
    Buckets(Vec<Bucket>),
    /// Lifts whose primary muscle is in the list.
    Muscles(Vec<Muscle>),
    /// Lifts whose movement pattern is in the list.
    Patterns(Vec<Pattern>),
    /// Every lift, shared out across all `Everything` days.
    Everything,
}

impl DayRule {
    pub fn matches(&self, tags: &LiftTags) -> bool {
        match self {
            DayRule::Buckets(b) => b.contains(&bucket(tags)),
            DayRule::Muscles(m) => m.contains(&tags.primary),
            DayRule::Patterns(p) => p.contains(&tags.pattern),
            DayRule::Everything => true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SplitDay {
    pub name: String,
    /// 0 = Monday … 6 = Sunday.
    pub weekday: u8,
    pub rule: DayRule,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Split {
    /// "ppl", "upper_lower", "bro", "full_body", or "custom:<anything>".
    pub key: String,
    pub name: String,
    pub summary: String,
    pub days: Vec<SplitDay>,
}

impl Split {
    /// Check a split written by a person or an agent before it is stored.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("The split needs a name".into());
        }
        if self.name.len() > 60 || self.key.len() > 40 || self.summary.len() > 200 {
            return Err("The split's key, name or summary is too long".into());
        }
        if self.days.is_empty() {
            return Err("A split needs at least one day".into());
        }
        if self.days.len() > 7 {
            return Err("A split can have at most seven days".into());
        }
        for d in &self.days {
            if d.name.trim().is_empty() {
                return Err("Every day needs a name".into());
            }
            if d.name.len() > 40 {
                return Err("A day's name can be at most 40 characters".into());
            }
            if d.weekday > 6 {
                return Err(format!("{}: weekday must be 0 (Monday) to 6 (Sunday)", d.name));
            }
            let empty = match &d.rule {
                DayRule::Buckets(v) => v.is_empty(),
                DayRule::Muscles(v) => v.is_empty(),
                DayRule::Patterns(v) => v.is_empty(),
                DayRule::Everything => false,
            };
            if empty {
                return Err(format!("{}: the rule lists nothing", d.name));
            }
        }
        let mut wd: Vec<u8> = self.days.iter().map(|d| d.weekday).collect();
        wd.sort_unstable();
        wd.dedup();
        if wd.len() != self.days.len() {
            return Err("Two days fall on the same weekday".into());
        }
        Ok(())
    }

    pub fn day_for_weekday(&self, weekday: u8) -> Option<usize> {
        self.days.iter().position(|d| d.weekday == weekday)
    }
}

fn day(name: &str, weekday: u8, rule: DayRule) -> SplitDay {
    SplitDay { name: name.into(), weekday, rule }
}

pub fn builtin() -> Vec<Split> {
    use Bucket as B;
    use Muscle as M;
    vec![
        Split {
            key: "ppl".into(),
            name: "Push, pull, legs".into(),
            summary: "Push, pull, legs · 3 days a week".into(),
            days: vec![
                day("Push", 0, DayRule::Buckets(vec![B::Push])),
                day("Pull", 2, DayRule::Buckets(vec![B::Pull])),
                day("Legs", 4, DayRule::Buckets(vec![B::Legs])),
            ],
        },
        Split {
            key: "upper_lower".into(),
            name: "Upper lower".into(),
            summary: "Upper and lower · 4 days a week".into(),
            days: vec![
                day("Upper A", 0, DayRule::Buckets(vec![B::Push, B::Pull])),
                day("Lower A", 1, DayRule::Buckets(vec![B::Legs])),
                day("Upper B", 3, DayRule::Buckets(vec![B::Push, B::Pull])),
                day("Lower B", 4, DayRule::Buckets(vec![B::Legs])),
            ],
        },
        Split {
            key: "bro".into(),
            name: "Bro split".into(),
            summary: "One muscle group a day · 5 days a week".into(),
            days: vec![
                day("Chest", 0, DayRule::Muscles(vec![M::Chest])),
                day("Back", 1, DayRule::Muscles(vec![M::Lats, M::MiddleBack, M::LowerBack, M::Traps])),
                day("Shoulders", 2, DayRule::Muscles(vec![M::Shoulders, M::Neck])),
                day("Arms", 3, DayRule::Muscles(vec![M::Biceps, M::Triceps, M::Forearms])),
                day("Legs", 4, DayRule::Muscles(vec![M::Quadriceps, M::Hamstrings, M::Glutes, M::Calves, M::Adductors, M::Abductors])),
            ],
        },
        Split {
            key: "full_body".into(),
            name: "Full body".into(),
            summary: "Full body · 3 days a week".into(),
            days: vec![
                day("Day A", 0, DayRule::Everything),
                day("Day B", 2, DayRule::Everything),
                day("Day C", 4, DayRule::Everything),
            ],
        },
    ]
}

pub fn builtin_by_key(key: &str) -> Option<Split> {
    builtin().into_iter().find(|s| s.key == key)
}

/// The built-in style a split came from: "ppl" for both "ppl" and "ppl@013457".
pub fn style_of(key: &str) -> &str {
    key.split('@').next().unwrap_or(key)
}

/// A built-in style spread over the weekdays you train (0 is Monday).
///
/// Push, pull, legs on six days is push, pull, legs twice; upper lower alternates; full
/// body gives every day everything; the bro split's five groups are shared out over fewer
/// days, or repeat over more. The default days give back the built-in split itself.
pub fn with_days(style: &str, weekdays: &[u8]) -> Option<Split> {
    use Bucket as B;
    use Muscle as M;
    let base = builtin_by_key(style)?;
    let mut wd: Vec<u8> = weekdays.iter().copied().filter(|d| *d <= 6).collect();
    wd.sort_unstable();
    wd.dedup();
    if wd.is_empty() {
        return None;
    }
    if base.days.iter().map(|d| d.weekday).collect::<Vec<_>>() == wd {
        return Some(base);
    }
    let n = wd.len();
    // Name repeats A, B, C when a pattern comes round again.
    let named = |names: &[&str], i: usize| -> String {
        let k = names.len();
        let name = names[i % k];
        if n > k { format!("{name} {}", ["A", "B", "C", "D", "E", "F", "G"][i / k]) } else { name.to_string() }
    };
    let days: Vec<SplitDay> = match style {
        "ppl" => {
            let rules = [B::Push, B::Pull, B::Legs];
            wd.iter().enumerate().map(|(i, w)| day(&named(&["Push", "Pull", "Legs"], i), *w, DayRule::Buckets(vec![rules[i % 3]]))).collect()
        }
        "upper_lower" => wd
            .iter()
            .enumerate()
            .map(|(i, w)| {
                let rule = if i % 2 == 0 { DayRule::Buckets(vec![B::Push, B::Pull]) } else { DayRule::Buckets(vec![B::Legs]) };
                day(&named(&["Upper", "Lower"], i), *w, rule)
            })
            .collect(),
        "full_body" => wd.iter().enumerate().map(|(i, w)| day(&format!("Day {}", ["A", "B", "C", "D", "E", "F", "G"][i]), *w, DayRule::Everything)).collect(),
        "bro" => {
            let groups: [(&str, Vec<Muscle>); 5] = [
                ("Chest", vec![M::Chest]),
                ("Back", vec![M::Lats, M::MiddleBack, M::LowerBack, M::Traps]),
                ("Shoulders", vec![M::Shoulders, M::Neck]),
                ("Arms", vec![M::Biceps, M::Triceps, M::Forearms]),
                ("Legs", vec![M::Quadriceps, M::Hamstrings, M::Glutes, M::Calves, M::Adductors, M::Abductors]),
            ];
            if n >= 5 {
                let names: Vec<&str> = groups.iter().map(|g| g.0).collect();
                wd.iter().enumerate().map(|(i, w)| day(&named(&names, i), *w, DayRule::Muscles(groups[i % 5].1.clone()))).collect()
            } else {
                // Fewer days than groups: deal the groups out in turn.
                let mut parts: Vec<(Vec<&str>, Vec<Muscle>)> = vec![(Vec::new(), Vec::new()); n];
                for (g, (name, muscles)) in groups.iter().enumerate() {
                    parts[g % n].0.push(name);
                    parts[g % n].1.extend(muscles.iter().copied());
                }
                wd.iter()
                    .zip(parts)
                    .map(|(w, (names, muscles))| {
                        let name = if n == 1 { "Whole body".to_string() } else { names.join(" and ") };
                        day(&name, *w, DayRule::Muscles(muscles))
                    })
                    .collect()
            }
        }
        _ => return None,
    };
    let code: String = wd.iter().map(|d| char::from(b'0' + d)).collect();
    let split = Split {
        key: format!("{style}@{code}"),
        name: base.name.clone(),
        summary: format!("{} · {n} day{} a week", base.name, if n == 1 { "" } else { "s" }),
        days,
    };
    split.validate().ok()?;
    Some(split)
}

/// One lift from the pool as the dealer sees it.
#[derive(Debug, Clone)]
pub struct PoolLift<'a> {
    pub id: Uuid,
    pub tags: Option<&'a LiftTags>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Dealt {
    /// Lift ids per day, in the order they will be trained.
    pub days: Vec<Vec<Uuid>>,
    /// Lifts that still need a look (no tags) and so stay out of the plan.
    pub unplaced: Vec<Uuid>,
}

/// Deal the pool into the split.
///
/// Compounds go first so the big lifts are spread before the accessories.
/// Each lift goes to the matching day that has the fewest lifts of the same bucket,
/// then the fewest lifts of the same kind (compound or isolation), then the fewest
/// lifts overall, then the earliest day. A tagged lift that matches no day (for
/// example an ab exercise in a push/pull/legs week) goes to the lightest day.
/// Untagged lifts are never guessed at.
pub fn deal(split: &Split, pool: &[PoolLift<'_>]) -> Dealt {
    let n = split.days.len();
    let mut out = Dealt { days: vec![Vec::new(); n], unplaced: Vec::new() };
    if n == 0 {
        out.unplaced = pool.iter().map(|l| l.id).collect();
        return out;
    }
    let mut ordered: Vec<(usize, &PoolLift<'_>)> = pool.iter().enumerate().collect();
    ordered.sort_by_key(|(i, l)| {
        let compound = l.tags.map(|t| t.mechanic == Mechanic::Compound).unwrap_or(false);
        (if compound { 0 } else { 1 }, *i)
    });

    // Per-day running counts.
    let mut buckets: Vec<Vec<Bucket>> = vec![Vec::new(); n];
    let mut compounds = vec![0usize; n];

    for (_, lift) in ordered {
        let Some(tags) = lift.tags else {
            out.unplaced.push(lift.id);
            continue;
        };
        let b = bucket(tags);
        let is_compound = tags.mechanic == Mechanic::Compound;
        let mut candidates: Vec<usize> = (0..n).filter(|&d| split.days[d].rule.matches(tags)).collect();
        if candidates.is_empty() {
            candidates = (0..n).collect();
        }
        let best = candidates
            .into_iter()
            .min_by_key(|&d| {
                let same_bucket = buckets[d].iter().filter(|x| **x == b).count();
                let same_kind = if is_compound { compounds[d] } else { out.days[d].len() - compounds[d] };
                (same_bucket, same_kind, out.days[d].len(), d)
            })
            .expect("at least one day");
        out.days[best].push(lift.id);
        buckets[best].push(b);
        if is_compound {
            compounds[best] += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::Catalogue;
    use crate::classify::classify;

    fn pool() -> Vec<(Uuid, String, LiftTags)> {
        let cat = Catalogue::from_json(include_str!("../../../data/catalogue/exercises.min.json")).unwrap();
        [
            "Bench press", "Overhead press", "Incline press", "Triceps pushdown", "Barbell row", "Pull-up",
            "Face pull", "Barbell curl", "Back squat", "Romanian deadlift", "Leg press", "Calf raise",
        ]
        .iter()
        .map(|n| (Uuid::new_v4(), n.to_string(), classify(n, &cat).tags.unwrap()))
        .collect()
    }

    fn names(p: &[(Uuid, String, LiftTags)], ids: &[Uuid]) -> Vec<String> {
        ids.iter().map(|id| p.iter().find(|x| x.0 == *id).unwrap().1.clone()).collect()
    }

    fn deal_named(key: &str) -> (Vec<Vec<String>>, Vec<Uuid>) {
        let p = pool();
        let lifts: Vec<PoolLift> = p.iter().map(|(id, _, t)| PoolLift { id: *id, tags: Some(t) }).collect();
        let d = deal(&builtin_by_key(key).unwrap(), &lifts);
        (d.days.iter().map(|ids| names(&p, ids)).collect(), d.unplaced)
    }

    fn sorted(mut v: Vec<String>) -> Vec<String> {
        v.sort();
        v
    }

    #[test]
    fn push_pull_legs_matches_the_screens() {
        let (days, unplaced) = deal_named("ppl");
        assert!(unplaced.is_empty());
        assert_eq!(sorted(days[0].clone()), sorted(vec!["Bench press".into(), "Overhead press".into(), "Incline press".into(), "Triceps pushdown".into()]));
        assert_eq!(sorted(days[1].clone()), sorted(vec!["Barbell row".into(), "Pull-up".into(), "Face pull".into(), "Barbell curl".into()]));
        assert_eq!(sorted(days[2].clone()), sorted(vec!["Back squat".into(), "Romanian deadlift".into(), "Leg press".into(), "Calf raise".into()]));
    }

    #[test]
    fn every_split_places_every_tagged_lift_exactly_once() {
        for s in builtin() {
            let (days, unplaced) = deal_named(&s.key);
            assert!(unplaced.is_empty(), "{}", s.key);
            let mut all: Vec<String> = days.concat();
            assert_eq!(all.len(), 12, "{}: {all:?}", s.key);
            all.sort();
            all.dedup();
            assert_eq!(all.len(), 12, "{} placed a lift twice", s.key);
            assert!(days.iter().all(|d| !d.is_empty()), "{} left a day empty: {days:?}", s.key);
        }
    }

    #[test]
    fn upper_lower_spreads_pushes_and_pulls_over_both_upper_days() {
        let (days, _) = deal_named("upper_lower");
        for upper in [&days[0], &days[2]] {
            assert!(upper.iter().any(|n| ["Bench press", "Overhead press", "Incline press"].contains(&n.as_str())), "{upper:?}");
            assert!(upper.iter().any(|n| ["Barbell row", "Pull-up"].contains(&n.as_str())), "{upper:?}");
        }
        assert!(days[1].iter().chain(days[3].iter()).all(|n| ["Back squat", "Romanian deadlift", "Leg press", "Calf raise"].contains(&n.as_str())));
    }

    #[test]
    fn full_body_gives_every_day_a_leg_push_and_pull() {
        let (days, _) = deal_named("full_body");
        let p = pool();
        for d in &days {
            let bs: Vec<Bucket> = d.iter().map(|n| bucket(&p.iter().find(|x| &x.1 == n).unwrap().2)).collect();
            for b in [Bucket::Push, Bucket::Pull, Bucket::Legs] {
                assert!(bs.contains(&b), "day {d:?} has no {b:?}");
            }
        }
    }

    #[test]
    fn bro_split_puts_lifts_on_their_muscle_day() {
        let (days, _) = deal_named("bro");
        assert!(days[0].contains(&"Bench press".to_string()));
        assert!(days[1].contains(&"Barbell row".to_string()));
        assert!(days[2].contains(&"Overhead press".to_string()));
        assert!(days[3].contains(&"Barbell curl".to_string()) && days[3].contains(&"Triceps pushdown".to_string()));
        assert!(days[4].contains(&"Back squat".to_string()));
    }

    #[test]
    fn untagged_lifts_stay_out_and_core_finds_a_home() {
        let cat = Catalogue::from_json(include_str!("../../../data/catalogue/exercises.min.json")).unwrap();
        let plank = classify("Plank", &cat).tags.unwrap();
        let bench = classify("Bench press", &cat).tags.unwrap();
        let (a, b, c) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let pool = vec![PoolLift { id: a, tags: Some(&bench) }, PoolLift { id: b, tags: None }, PoolLift { id: c, tags: Some(&plank) }];
        let d = deal(&builtin_by_key("ppl").unwrap(), &pool);
        assert_eq!(d.unplaced, vec![b]);
        assert_eq!(d.days.concat().len(), 2);
        assert!(!d.days[0].contains(&c), "plank should go to a lighter day than push");
    }

    #[test]
    fn validation_refuses_broken_splits() {
        let mut s = builtin_by_key("ppl").unwrap();
        assert!(s.validate().is_ok());
        s.days[1].weekday = 0;
        assert!(s.validate().is_err());
        let mut s = builtin_by_key("ppl").unwrap();
        s.days[0].rule = DayRule::Muscles(vec![]);
        assert!(s.validate().is_err());
        let json = r#"{"key":"custom:x","name":"X","summary":"","days":[{"name":"A","weekday":0,"rule":{"kind":"buckets","values":["push","sideways"]}}]}"#;
        assert!(serde_json::from_str::<Split>(json).is_err(), "unknown bucket must be refused");
    }

    #[test]
    fn any_number_of_training_days() {
        let six = with_days("ppl", &[0, 1, 2, 3, 4, 5]).unwrap();
        assert_eq!(six.days.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["Push A", "Pull A", "Legs A", "Push B", "Pull B", "Legs B"]);
        assert_eq!(six.summary, "Push, pull, legs · 6 days a week");
        assert_eq!(style_of(&six.key), "ppl");
        // The default days give the built-in split back, key and all.
        assert_eq!(with_days("ppl", &[4, 0, 2]).unwrap().key, "ppl");
        let two = with_days("bro", &[1, 4]).unwrap();
        assert_eq!(two.days.len(), 2);
        assert!(two.validate().is_ok());
        for style in ["ppl", "upper_lower", "full_body", "bro"] {
            for n in 1..=7u8 {
                let days: Vec<u8> = (0..n).collect();
                let s = with_days(style, &days).unwrap_or_else(|| panic!("{style} on {n} days"));
                assert_eq!(s.days.len(), n as usize, "{style} on {n} days");
            }
        }
        assert!(with_days("ppl", &[]).is_none());
    }
}
