//! The public-domain exercise catalogue (free-exercise-db), indexed by aliases.

use std::collections::HashMap;

use serde::Deserialize;

use crate::classify::{normalise, pattern_from_catalogue};
use crate::tags::{Equipment, LiftTags, Mechanic, Muscle};

#[derive(Debug, Deserialize)]
struct Raw {
    name: String,
    force: Option<String>,
    mechanic: Option<String>,
    equipment: Option<String>,
    primary: Vec<String>,
    secondary: Vec<String>,
    category: String,
}

#[derive(Debug, Clone)]
struct Indexed {
    tags: LiftTags,
    /// Lower is better: exact full-name matches and plain strength entries win.
    rank: u8,
}

#[derive(Debug, Default, Clone)]
pub struct Catalogue {
    by_alias: HashMap<String, Indexed>,
    by_tokens: HashMap<String, Indexed>,
    len: usize,
}

const EQUIPMENT_WORDS: [&str; 8] = ["barbell", "dumbbell", "cable", "kettlebell", "machine", "smith machine", "ez", "band"];

impl Catalogue {
    pub fn from_json(json: &str) -> Result<Catalogue, serde_json::Error> {
        let raw: Vec<Raw> = serde_json::from_str(json)?;
        let mut cat = Catalogue::default();
        for r in raw {
            let Some(primary) = r.primary.first().and_then(|m| Muscle::from_catalogue(m)) else {
                continue;
            };
            let secondary: Vec<Muscle> = r
                .secondary
                .iter()
                .filter_map(|m| Muscle::from_catalogue(m))
                .filter(|m| *m != primary)
                .collect();
            let mechanic = match r.mechanic.as_deref() {
                Some("isolation") => Mechanic::Isolation,
                Some("compound") => Mechanic::Compound,
                _ if secondary.is_empty() => Mechanic::Isolation,
                _ => Mechanic::Compound,
            };
            let full = normalise(&r.name);
            let pattern = pattern_from_catalogue(r.force.as_deref(), mechanic, primary, &full);
            let tags = LiftTags { pattern, primary, secondary, equipment: Equipment::from_catalogue(r.equipment.as_deref()), mechanic };
            let base_rank: u8 = match r.category.as_str() {
                "strength" => 0,
                "powerlifting" => 2,
                _ => 4,
            };
            cat.len += 1;
            for (alias, extra) in aliases(&r.name) {
                cat.insert(alias, tags.clone(), base_rank + extra);
            }
        }
        Ok(cat)
    }

    fn insert(&mut self, alias: String, tags: LiftTags, rank: u8) {
        if alias.is_empty() {
            return;
        }
        let token_key = token_key(&alias);
        let entry = Indexed { tags, rank };
        for (map, key) in [(&mut self.by_alias, alias), (&mut self.by_tokens, token_key)] {
            match map.get(&key) {
                Some(existing) if existing.rank <= entry.rank => {}
                _ => {
                    map.insert(key, entry.clone());
                }
            }
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Look up an already normalised name.
    pub fn lookup(&self, norm: &str) -> Option<&LiftTags> {
        self.by_alias
            .get(norm)
            .or_else(|| self.by_tokens.get(&token_key(norm)))
            .map(|e| &e.tags)
    }
}

/// Word order does not matter: "dumbbell incline press" == "incline dumbbell press".
fn token_key(norm: &str) -> String {
    let mut words: Vec<&str> = norm.split_whitespace().collect();
    words.sort_unstable();
    words.dedup();
    words.join(" ")
}

/// Names a person might type for a catalogue entry, with a rank penalty for looser ones.
fn aliases(name: &str) -> Vec<(String, u8)> {
    let mut out = vec![(normalise(name), 0u8)];
    // Variants with bands or chains should not stand in for the plain lift.
    let lower = name.to_ascii_lowercase();
    let variant = ["band", "chain", "reverse band", "with bands", "with chains"].iter().any(|w| lower.contains(w));
    let penalty = if variant { 4 } else { 0 };
    // "Barbell Bench Press - Medium Grip" -> "Barbell Bench Press"
    let head = name.split(" - ").next().unwrap_or(name);
    // "Seated Cable Rows (narrow)" -> without the parenthesis
    let head = head.split('(').next().unwrap_or(head);
    let head_norm = normalise(head);
    out.push((head_norm.clone(), 1 + penalty));
    // Drop a leading or trailing equipment word: "barbell bench press" -> "bench press".
    for w in EQUIPMENT_WORDS {
        if let Some(rest) = head_norm.strip_prefix(&format!("{w} ")) {
            out.push((rest.to_string(), 2 + penalty));
        }
        if let Some(rest) = head_norm.strip_suffix(&format!(" {w}")) {
            out.push((rest.to_string(), 2 + penalty));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_and_finds_common_lifts() {
        let c = Catalogue::from_json(include_str!("../../../data/catalogue/exercises.min.json")).unwrap();
        assert!(c.len() > 600, "only {} entries", c.len());
        for name in ["bench press", "barbell squat", "romanian deadlift", "dumbbell bench press", "incline dumbbell press", "pullups"] {
            assert!(c.lookup(&normalise(name)).is_some(), "{name} missing from catalogue");
        }
        let bench = c.lookup("bench press").unwrap();
        assert_eq!(bench.primary, Muscle::Chest);
    }
}
