//! The tags every lift carries. Values are closed sets: an agent that sends
//! anything else is refused at parse time.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Pattern {
    HorizontalPush,
    VerticalPush,
    HorizontalPull,
    VerticalPull,
    Squat,
    Hinge,
    Lunge,
    Carry,
    Core,
    Isolation,
}

impl Pattern {
    pub const ALL: [Pattern; 10] = [
        Pattern::HorizontalPush,
        Pattern::VerticalPush,
        Pattern::HorizontalPull,
        Pattern::VerticalPull,
        Pattern::Squat,
        Pattern::Hinge,
        Pattern::Lunge,
        Pattern::Carry,
        Pattern::Core,
        Pattern::Isolation,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Pattern::HorizontalPush => "Horizontal push",
            Pattern::VerticalPush => "Vertical push",
            Pattern::HorizontalPull => "Horizontal pull",
            Pattern::VerticalPull => "Vertical pull",
            Pattern::Squat => "Squat",
            Pattern::Hinge => "Hinge",
            Pattern::Lunge => "Lunge",
            Pattern::Carry => "Carry",
            Pattern::Core => "Core",
            Pattern::Isolation => "Isolation",
        }
    }
}

/// Muscle names follow the free-exercise-db catalogue so its entries map one to one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Muscle {
    Chest,
    Shoulders,
    Triceps,
    Lats,
    MiddleBack,
    Traps,
    Biceps,
    Forearms,
    LowerBack,
    Quadriceps,
    Hamstrings,
    Glutes,
    Calves,
    Adductors,
    Abductors,
    Abdominals,
    Neck,
}

impl Muscle {
    pub const ALL: [Muscle; 17] = [
        Muscle::Chest,
        Muscle::Shoulders,
        Muscle::Triceps,
        Muscle::Lats,
        Muscle::MiddleBack,
        Muscle::Traps,
        Muscle::Biceps,
        Muscle::Forearms,
        Muscle::LowerBack,
        Muscle::Quadriceps,
        Muscle::Hamstrings,
        Muscle::Glutes,
        Muscle::Calves,
        Muscle::Adductors,
        Muscle::Abductors,
        Muscle::Abdominals,
        Muscle::Neck,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Muscle::Chest => "Chest",
            Muscle::Shoulders => "Shoulders",
            Muscle::Triceps => "Triceps",
            Muscle::Lats => "Lats",
            Muscle::MiddleBack => "Middle back",
            Muscle::Traps => "Traps",
            Muscle::Biceps => "Biceps",
            Muscle::Forearms => "Forearms",
            Muscle::LowerBack => "Lower back",
            Muscle::Quadriceps => "Quads",
            Muscle::Hamstrings => "Hamstrings",
            Muscle::Glutes => "Glutes",
            Muscle::Calves => "Calves",
            Muscle::Adductors => "Adductors",
            Muscle::Abductors => "Abductors",
            Muscle::Abdominals => "Abs",
            Muscle::Neck => "Neck",
        }
    }

    /// Parse the catalogue's spelling ("middle back", "quadriceps").
    pub fn from_catalogue(s: &str) -> Option<Muscle> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "chest" => Muscle::Chest,
            "shoulders" => Muscle::Shoulders,
            "triceps" => Muscle::Triceps,
            "lats" => Muscle::Lats,
            "middle back" => Muscle::MiddleBack,
            "traps" => Muscle::Traps,
            "biceps" => Muscle::Biceps,
            "forearms" => Muscle::Forearms,
            "lower back" => Muscle::LowerBack,
            "quadriceps" => Muscle::Quadriceps,
            "hamstrings" => Muscle::Hamstrings,
            "glutes" => Muscle::Glutes,
            "calves" => Muscle::Calves,
            "adductors" => Muscle::Adductors,
            "abductors" => Muscle::Abductors,
            "abdominals" => Muscle::Abdominals,
            "neck" => Muscle::Neck,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Equipment {
    Barbell,
    Dumbbell,
    Cable,
    Machine,
    Bodyweight,
    Kettlebell,
    Band,
    EzBar,
    Other,
}

impl Equipment {
    pub const ALL: [Equipment; 9] = [
        Equipment::Barbell,
        Equipment::Dumbbell,
        Equipment::Cable,
        Equipment::Machine,
        Equipment::Bodyweight,
        Equipment::Kettlebell,
        Equipment::Band,
        Equipment::EzBar,
        Equipment::Other,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Equipment::Barbell => "Barbell",
            Equipment::Dumbbell => "Dumbbell",
            Equipment::Cable => "Cable",
            Equipment::Machine => "Machine",
            Equipment::Bodyweight => "Bodyweight",
            Equipment::Kettlebell => "Kettlebell",
            Equipment::Band => "Band",
            Equipment::EzBar => "EZ bar",
            Equipment::Other => "Other",
        }
    }

    pub fn from_catalogue(s: Option<&str>) -> Equipment {
        match s.map(|s| s.trim().to_ascii_lowercase()) {
            Some(s) => match s.as_str() {
                "barbell" => Equipment::Barbell,
                "dumbbell" => Equipment::Dumbbell,
                "cable" => Equipment::Cable,
                "machine" => Equipment::Machine,
                "body only" => Equipment::Bodyweight,
                "kettlebells" => Equipment::Kettlebell,
                "bands" => Equipment::Band,
                "e-z curl bar" => Equipment::EzBar,
                _ => Equipment::Other,
            },
            None => Equipment::Other,
        }
    }

    /// Does the barbell picture make sense for this lift?
    pub fn uses_plates(self) -> bool {
        matches!(self, Equipment::Barbell | Equipment::EzBar)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mechanic {
    Compound,
    Isolation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiftTags {
    pub pattern: Pattern,
    pub primary: Muscle,
    #[serde(default)]
    pub secondary: Vec<Muscle>,
    pub equipment: Equipment,
    pub mechanic: Mechanic,
}

/// Who decided a lift's tags. Order matters: a later source never overwrites `User`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TagSource {
    Catalogue,
    Rules,
    Agent,
    User,
}

impl TagSource {
    pub fn as_str(self) -> &'static str {
        match self {
            TagSource::Catalogue => "catalogue",
            TagSource::Rules => "rules",
            TagSource::Agent => "agent",
            TagSource::User => "user",
        }
    }

    pub fn parse(s: &str) -> Option<TagSource> {
        Some(match s {
            "catalogue" => TagSource::Catalogue,
            "rules" => TagSource::Rules,
            "agent" => TagSource::Agent,
            "user" => TagSource::User,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            TagSource::Catalogue => "Matched the catalogue",
            TagSource::Rules => "Sorted by name",
            TagSource::Agent => "Tagged by an agent",
            TagSource::User => "Set by you",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_tag_values_are_refused() {
        let ok = r#"{"pattern":"hinge","primary":"hamstrings","secondary":["glutes"],"equipment":"barbell","mechanic":"compound"}"#;
        assert!(serde_json::from_str::<LiftTags>(ok).is_ok());
        let bad = r#"{"pattern":"hinge","primary":"posterior_chain","equipment":"barbell","mechanic":"compound"}"#;
        assert!(serde_json::from_str::<LiftTags>(bad).is_err());
        let bad_pattern = r#"{"pattern":"twist","primary":"abdominals","equipment":"cable","mechanic":"isolation"}"#;
        assert!(serde_json::from_str::<LiftTags>(bad_pattern).is_err());
    }
}
