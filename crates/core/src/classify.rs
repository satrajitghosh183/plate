//! Automatic sorting of lifts.
//!
//! Order, stopping at the first confident answer (SPEC section 5):
//! 1. the catalogue, by normalised name and aliases;
//! 2. keyword rules on the name;
//! 3. tags an agent supplies (validated by the closed enums in `tags`);
//! 4. the user's own correction, which always wins and is never overwritten.
//!
//! Steps 3 and 4 live in the backend because they need storage; this module owns
//! 1 and 2 and the precedence rule (`may_overwrite`).

use crate::catalogue::Catalogue;
use crate::tags::{Equipment, LiftTags, Mechanic, Muscle, Pattern, TagSource};

#[derive(Debug, Clone, PartialEq)]
pub struct Classification {
    pub tags: Option<LiftTags>,
    pub source: Option<TagSource>,
}

impl Classification {
    pub fn needs_look(&self) -> bool {
        self.tags.is_none()
    }
}

/// Run steps 1 and 2.
pub fn classify(name: &str, catalogue: &Catalogue) -> Classification {
    let norm = normalise(name);
    let ruled = rules(&norm);
    if let Some(entry) = catalogue.lookup(&norm) {
        // The catalogue knows muscles and equipment well, but its force field is
        // too coarse for movement patterns, so the name rules decide the pattern.
        let mut tags = entry.clone();
        if let Some(r) = &ruled {
            tags.pattern = r.pattern;
            if tags.mechanic == Mechanic::Isolation && r.mechanic == Mechanic::Compound {
                tags.mechanic = Mechanic::Compound;
            }
        }
        // Equipment: what the name says, else what the movement implies, else the catalogue.
        if let Some(eq) = equipment_in_name(&norm).or_else(|| ruled.as_ref().map(|r| r.equipment)) {
            tags.equipment = eq;
        }
        return Classification { tags: Some(tags), source: Some(TagSource::Catalogue) };
    }
    match ruled {
        Some(tags) => Classification { tags: Some(tags), source: Some(TagSource::Rules) },
        None => Classification { tags: None, source: None },
    }
}

/// May tags from `incoming` replace tags from `existing`? Only the user can
/// replace the user's tags.
pub fn may_overwrite(existing: Option<TagSource>, incoming: TagSource) -> bool {
    match existing {
        Some(TagSource::User) => incoming == TagSource::User,
        _ => true,
    }
}

/// Lowercase, strip punctuation, expand gym shorthand, singularise common words.
pub fn normalise(name: &str) -> String {
    let mut s = String::with_capacity(name.len());
    for ch in name.chars() {
        match ch {
            'a'..='z' | '0'..='9' => s.push(ch),
            'A'..='Z' => s.push(ch.to_ascii_lowercase()),
            '-' | '_' | '/' | ',' | '.' | '(' | ')' | '&' | '+' | '\t' | ' ' => s.push(' '),
            '\'' | '’' => {}
            _ if ch.is_alphanumeric() => s.extend(ch.to_lowercase()),
            _ => s.push(' '),
        }
    }
    let mut out: Vec<String> = Vec::new();
    for word in s.split_whitespace() {
        let mapped: &str = match word {
            "db" | "dbs" | "dumbbells" => "dumbbell",
            "bb" => "barbell",
            "kb" | "kettlebells" => "kettlebell",
            "ohp" => "overhead press",
            "rdl" | "rdls" => "romanian deadlift",
            "sldl" => "stiff leg deadlift",
            "pullup" | "pullups" => "pull up",
            "chinup" | "chinups" => "chin up",
            "pushup" | "pushups" => "push up",
            "situp" | "situps" => "sit up",
            "tricep" => "triceps",
            "bicep" => "biceps",
            "flye" | "flyes" | "flys" | "flies" => "fly",
            "raises" => "raise",
            "curls" => "curl",
            "rows" => "row",
            "presses" => "press",
            "extensions" => "extension",
            "squats" => "squat",
            "lunges" => "lunge",
            "deadlifts" => "deadlift",
            "pulldowns" => "pulldown",
            "pushdowns" => "pushdown",
            "dips" => "dip",
            "shrugs" => "shrug",
            "crunches" => "crunch",
            "kickbacks" => "kickback",
            "thrusts" => "thrust",
            "bridges" => "bridge",
            "planks" => "plank",
            "carries" => "carry",
            "swings" => "swing",
            "rollouts" => "rollout",
            "crossovers" => "crossover",
            "skullcrusher" | "skullcrushers" | "crushers" => "skull crusher",
            "crusher" => "crusher",
            "stepup" | "stepups" => "step up",
            "facepull" | "facepulls" => "face pull",
            "pulls" => "pull",
            "ups" => "up",
            "hyperextensions" => "hyperextension",
            "ez" | "ezbar" => "ez",
            "w" | "with" | "the" | "a" | "on" | "using" => continue,
            other => other,
        };
        out.push(mapped.to_string());
    }
    let joined = out.join(" ");
    // "skull crusher" may have doubled when the input already said "skull crusher".
    joined.replace("skull skull crusher", "skull crusher").replace("skull crusher crusher", "skull crusher")
}

fn has(norm: &str, phrase: &str) -> bool {
    let hay = format!(" {norm} ");
    let needle = format!(" {phrase} ");
    hay.contains(&needle)
}

fn has_any(norm: &str, phrases: &[&str]) -> bool {
    phrases.iter().any(|p| has(norm, p))
}

/// Equipment named outright in the lift's name.
pub fn equipment_in_name(norm: &str) -> Option<Equipment> {
    if has(norm, "cable") {
        Some(Equipment::Cable)
    } else if has(norm, "dumbbell") {
        Some(Equipment::Dumbbell)
    } else if has(norm, "kettlebell") {
        Some(Equipment::Kettlebell)
    } else if has_any(norm, &["band", "banded", "bands"]) {
        Some(Equipment::Band)
    } else if has_any(norm, &["ez", "ez bar"]) {
        Some(Equipment::EzBar)
    } else if has_any(norm, &["smith", "machine"]) {
        Some(Equipment::Machine)
    } else if has_any(norm, &["barbell", "trap bar", "hex bar"]) {
        Some(Equipment::Barbell)
    } else if has_any(norm, &["bodyweight", "body weight"]) {
        Some(Equipment::Bodyweight)
    } else {
        None
    }
}

/// Equipment implied by the movement itself when the name does not say.
fn implied_equipment(norm: &str) -> Option<Equipment> {
    if has_any(norm, &["pulldown", "pushdown", "face pull", "crossover", "cable cross", "pallof", "woodchop", "wood chop", "cable crunch"]) {
        Some(Equipment::Cable)
    } else if has_any(norm, &["leg press", "hack squat", "pec deck", "leg extension", "leg curl", "chest press", "seated calf", "adductor", "abductor"]) {
        Some(Equipment::Machine)
    } else if has_any(norm, &["pull up", "chin up", "push up", "dip", "plank", "sit up", "crunch", "leg raise", "knee raise", "hanging", "inverted row", "nordic", "dead bug", "hollow", "v up", "toes to bar", "dragon flag", "back extension", "hyperextension"]) {
        Some(Equipment::Bodyweight)
    } else {
        None
    }
}

fn tags(pattern: Pattern, primary: Muscle, secondary: &[Muscle], equipment: Equipment, mechanic: Mechanic) -> LiftTags {
    LiftTags { pattern, primary, secondary: secondary.to_vec(), equipment, mechanic }
}

/// Step 2: keyword rules. Most specific first.
pub fn rules(norm: &str) -> Option<LiftTags> {
    use Equipment as E;
    use Mechanic::{Compound, Isolation};
    use Muscle as M;
    use Pattern as P;

    let n = norm;
    let mut t: Option<LiftTags> = None;

    if has_any(n, &["calf raise", "calf press", "calf raises"]) {
        t = Some(tags(P::Isolation, M::Calves, &[], E::Machine, Isolation));
    } else if has_any(n, &["leg curl", "hamstring curl", "nordic", "nordic curl"]) {
        t = Some(tags(P::Isolation, M::Hamstrings, &[M::Calves], E::Machine, Isolation));
    } else if has(n, "leg extension") {
        t = Some(tags(P::Isolation, M::Quadriceps, &[], E::Machine, Isolation));
    } else if has_any(n, &["hip thrust", "glute bridge", "hip bridge"]) {
        t = Some(tags(P::Hinge, M::Glutes, &[M::Hamstrings], E::Barbell, Compound));
    } else if has(n, "kickback") && has_any(n, &["glute", "hip"]) {
        t = Some(tags(P::Isolation, M::Glutes, &[M::Hamstrings], E::Cable, Isolation));
    } else if has_any(n, &["romanian deadlift", "stiff leg deadlift", "stiff legged deadlift", "good morning", "single leg deadlift"]) {
        t = Some(tags(P::Hinge, M::Hamstrings, &[M::Glutes, M::LowerBack], E::Barbell, Compound));
    } else if has(n, "deadlift") {
        t = Some(tags(P::Hinge, M::Hamstrings, &[M::Glutes, M::LowerBack, M::Traps, M::Forearms], E::Barbell, Compound));
    } else if has_any(n, &["back extension", "hyperextension", "reverse hyper"]) {
        t = Some(tags(P::Hinge, M::LowerBack, &[M::Glutes, M::Hamstrings], E::Bodyweight, Isolation));
    } else if has(n, "swing") {
        t = Some(tags(P::Hinge, M::Glutes, &[M::Hamstrings, M::LowerBack], E::Kettlebell, Compound));
    } else if has_any(n, &["clean", "snatch", "power clean", "hang clean"]) && !has(n, "jerk") {
        t = Some(tags(P::Hinge, M::Hamstrings, &[M::Quadriceps, M::Glutes, M::Traps], E::Barbell, Compound));
    } else if has(n, "jerk") {
        t = Some(tags(P::VerticalPush, M::Shoulders, &[M::Triceps, M::Quadriceps], E::Barbell, Compound));
    } else if has_any(n, &["split squat", "lunge", "step up", "bulgarian"]) {
        t = Some(tags(P::Lunge, M::Quadriceps, &[M::Glutes, M::Hamstrings], E::Dumbbell, Compound));
    } else if has_any(n, &["squat", "leg press", "hack"]) {
        t = Some(tags(P::Squat, M::Quadriceps, &[M::Glutes, M::Hamstrings], E::Barbell, Compound));
    } else if has_any(n, &["adductor", "adduction", "hip adduction"]) {
        t = Some(tags(P::Isolation, M::Adductors, &[], E::Machine, Isolation));
    } else if has_any(n, &["abductor", "abduction", "hip abduction"]) {
        t = Some(tags(P::Isolation, M::Abductors, &[], E::Machine, Isolation));
    } else if has(n, "shrug") {
        t = Some(tags(P::Isolation, M::Traps, &[], E::Dumbbell, Isolation));
    } else if has(n, "face pull") {
        t = Some(tags(P::HorizontalPull, M::Shoulders, &[M::Traps, M::MiddleBack], E::Cable, Isolation));
    } else if has_any(n, &["rear delt", "reverse fly", "rear fly", "reverse pec deck"]) {
        t = Some(tags(P::Isolation, M::Shoulders, &[M::MiddleBack, M::Traps], E::Dumbbell, Isolation));
    } else if has(n, "upright row") {
        t = Some(tags(P::VerticalPull, M::Shoulders, &[M::Traps], E::Barbell, Compound));
    } else if has_any(n, &["straight arm pulldown", "pullover"]) {
        t = Some(tags(P::Isolation, M::Lats, &[M::Chest], E::Cable, Isolation));
    } else if has_any(n, &["pulldown", "pull up", "chin up", "lat pull"]) {
        t = Some(tags(P::VerticalPull, M::Lats, &[M::Biceps, M::MiddleBack], E::Bodyweight, Compound));
    } else if has(n, "row") {
        t = Some(tags(P::HorizontalPull, M::MiddleBack, &[M::Lats, M::Biceps], E::Barbell, Compound));
    } else if has_any(n, &["lateral raise", "side raise", "front raise", "y raise", "lu raise", "delt raise"]) {
        t = Some(tags(P::Isolation, M::Shoulders, &[], E::Dumbbell, Isolation));
    } else if has_any(n, &["skull crusher", "pushdown", "triceps extension", "overhead extension", "french press", "lying extension", "jm press"])
        || (has(n, "kickback"))
        || (has(n, "triceps") && has_any(n, &["extension", "press down", "rope"]))
    {
        t = Some(tags(P::Isolation, M::Triceps, &[], E::Dumbbell, Isolation));
    } else if has_any(n, &["close grip bench", "close grip press", "close grip bench press"]) {
        t = Some(tags(P::HorizontalPush, M::Triceps, &[M::Chest, M::Shoulders], E::Barbell, Compound));
    } else if has_any(n, &["triceps dip", "bench dip"]) {
        t = Some(tags(P::HorizontalPush, M::Triceps, &[M::Chest, M::Shoulders], E::Bodyweight, Compound));
    } else if has_any(n, &["overhead press", "shoulder press", "military press", "push press", "arnold press", "z press", "landmine press", "seated press"]) {
        t = Some(tags(P::VerticalPush, M::Shoulders, &[M::Triceps], E::Barbell, Compound));
    } else if has_any(n, &["fly", "pec deck", "crossover", "cable cross", "pec fly"]) {
        t = Some(tags(P::Isolation, M::Chest, &[M::Shoulders], E::Dumbbell, Isolation));
    } else if has_any(n, &["dip", "chest dip"]) {
        t = Some(tags(P::HorizontalPush, M::Chest, &[M::Triceps, M::Shoulders], E::Bodyweight, Compound));
    } else if has_any(n, &["bench press", "chest press", "floor press", "incline press", "decline press", "push up", "svend press", "bench"]) {
        t = Some(tags(P::HorizontalPush, M::Chest, &[M::Shoulders, M::Triceps], E::Barbell, Compound));
    } else if has_any(n, &["wrist curl", "reverse wrist curl", "wrist roller", "grip"]) {
        t = Some(tags(P::Isolation, M::Forearms, &[], E::Dumbbell, Isolation));
    } else if has(n, "curl") {
        t = Some(tags(P::Isolation, M::Biceps, &[M::Forearms], E::Dumbbell, Isolation));
    } else if has_any(n, &["farmer", "farmers walk", "carry", "suitcase", "yoke"]) {
        t = Some(tags(P::Carry, M::Forearms, &[M::Traps, M::Abdominals], E::Dumbbell, Compound));
    } else if has_any(n, &["plank", "crunch", "sit up", "leg raise", "knee raise", "ab wheel", "rollout", "pallof", "woodchop", "wood chop", "dead bug", "hollow", "russian twist", "oblique", "side bend", "v up", "toes to bar", "dragon flag", "ab", "abs", "core"]) {
        t = Some(tags(P::Core, M::Abdominals, &[], E::Bodyweight, Isolation));
    } else if has(n, "neck") {
        t = Some(tags(P::Isolation, M::Neck, &[], E::Other, Isolation));
    } else if has(n, "press") && has_any(n, &["incline", "decline", "chest"]) {
        t = Some(tags(P::HorizontalPush, M::Chest, &[M::Shoulders, M::Triceps], E::Barbell, Compound));
    }

    let mut t = t?;
    if let Some(eq) = equipment_in_name(n).or_else(|| implied_equipment(n)) {
        t.equipment = eq;
    }
    // Machines and cables replace the barbell default; bodyweight pulls stay bodyweight.
    Some(t)
}

/// A pattern from the catalogue's own fields, used when no name rule matched.
pub fn pattern_from_catalogue(force: Option<&str>, mechanic: Mechanic, primary: Muscle, norm: &str) -> Pattern {
    use Muscle as M;
    use Pattern as P;
    if primary == M::Abdominals {
        return P::Core;
    }
    if has_any(norm, &["carry", "farmer", "walk", "yoke"]) {
        return P::Carry;
    }
    if mechanic == Mechanic::Isolation {
        return P::Isolation;
    }
    match (force, primary) {
        (_, M::Quadriceps) if has_any(norm, &["lunge", "split", "step"]) => P::Lunge,
        (_, M::Quadriceps) => P::Squat,
        (_, M::Hamstrings | M::Glutes | M::LowerBack) => P::Hinge,
        (Some("push"), M::Shoulders) => P::VerticalPush,
        (Some("push"), _) => P::HorizontalPush,
        (Some("pull"), M::Lats) if has_any(norm, &["pull", "chin", "pulldown"]) => P::VerticalPull,
        (Some("pull"), M::Shoulders | M::Traps) => P::VerticalPull,
        (Some("pull"), _) => P::HorizontalPull,
        _ => P::Isolation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::Catalogue;

    fn cat() -> Catalogue {
        Catalogue::from_json(include_str!("../../../data/catalogue/exercises.min.json")).expect("catalogue parses")
    }

    fn sort(name: &str) -> LiftTags {
        classify(name, &cat()).tags.unwrap_or_else(|| panic!("{name} was not sorted"))
    }

    #[test]
    fn normalises_shorthand() {
        assert_eq!(normalise("DB Incline Press"), "dumbbell incline press");
        assert_eq!(normalise("RDLs"), "romanian deadlift");
        assert_eq!(normalise("Pull-ups"), "pull up");
        assert_eq!(normalise("OHP"), "overhead press");
        assert_eq!(normalise("Tricep Pushdowns"), "triceps pushdown");
        assert_eq!(normalise("Skullcrushers"), "skull crusher");
    }

    #[test]
    fn sorts_the_twelve_lifts_in_the_screens() {
        use Muscle as M;
        use Pattern as P;
        let cases: &[(&str, P, M)] = &[
            ("Bench press", P::HorizontalPush, M::Chest),
            ("Overhead press", P::VerticalPush, M::Shoulders),
            ("Incline press", P::HorizontalPush, M::Chest),
            ("Triceps pushdown", P::Isolation, M::Triceps),
            ("Barbell row", P::HorizontalPull, M::MiddleBack),
            ("Pull-up", P::VerticalPull, M::Lats),
            ("Face pull", P::HorizontalPull, M::Shoulders),
            ("Barbell curl", P::Isolation, M::Biceps),
            ("Back squat", P::Squat, M::Quadriceps),
            ("Romanian deadlift", P::Hinge, M::Hamstrings),
            ("Leg press", P::Squat, M::Quadriceps),
            ("Calf raise", P::Isolation, M::Calves),
        ];
        for (name, pattern, muscle) in cases {
            let t = sort(name);
            assert_eq!((t.pattern, t.primary), (*pattern, *muscle), "{name}: got {t:?}");
        }
    }

    #[test]
    fn sorts_more_names_from_the_screens() {
        for (name, eq) in [("Bench press", Equipment::Barbell), ("Back squat", Equipment::Barbell), ("Romanian deadlift", Equipment::Barbell), ("Overhead press", Equipment::Barbell), ("Leg press", Equipment::Machine), ("Pull-up", Equipment::Bodyweight), ("Dumbbell bench press", Equipment::Dumbbell), ("Banded bench press", Equipment::Band)] {
            assert_eq!(sort(name).equipment, eq, "{name}");
        }
        assert_eq!(sort("Lateral raise").primary, Muscle::Shoulders);
        assert_eq!(sort("Lateral raise").pattern, Pattern::Isolation);
        assert_eq!(sort("Cable fly").primary, Muscle::Chest);
        assert_eq!(sort("Cable fly").equipment, Equipment::Cable);
        assert_eq!(sort("Incline dumbbell press").equipment, Equipment::Dumbbell);
        assert_eq!(sort("Hip thrust").primary, Muscle::Glutes);
        assert_eq!(sort("Bulgarian split squat").pattern, Pattern::Lunge);
        assert_eq!(sort("Hammer curls").primary, Muscle::Biceps);
        assert_eq!(sort("Lat pulldown").equipment, Equipment::Cable);
        assert_eq!(sort("Hanging leg raise").pattern, Pattern::Core);
        assert_eq!(sort("Farmer's walk").pattern, Pattern::Carry);
        assert_eq!(sort("Leg curl").primary, Muscle::Hamstrings);
        assert_eq!(sort("Leg extension").primary, Muscle::Quadriceps);
    }

    #[test]
    fn unknown_names_need_a_look() {
        let c = classify("Zottman thing on the weird machine", &cat());
        // Either the catalogue knows it or it waits for the user; never a guess from nothing.
        if c.tags.is_none() {
            assert!(c.needs_look());
        }
        assert!(classify("Wibble", &cat()).needs_look());
        assert!(classify("Morning routine", &cat()).needs_look());
    }

    #[test]
    fn user_tags_are_never_overwritten() {
        assert!(!may_overwrite(Some(TagSource::User), TagSource::Agent));
        assert!(!may_overwrite(Some(TagSource::User), TagSource::Catalogue));
        assert!(!may_overwrite(Some(TagSource::User), TagSource::Rules));
        assert!(may_overwrite(Some(TagSource::User), TagSource::User));
        assert!(may_overwrite(Some(TagSource::Rules), TagSource::Agent));
        assert!(may_overwrite(None, TagSource::Agent));
    }
}
