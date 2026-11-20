//! Weights are stored in grams so kg and lb never round-trip through floats.

use serde::{Deserialize, Serialize};

pub const GRAMS_PER_LB: f64 = 453.592_37;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Unit {
    #[default]
    Kg,
    Lb,
}

impl Unit {
    pub fn label(self) -> &'static str {
        match self {
            Unit::Kg => "kg",
            Unit::Lb => "lb",
        }
    }

    /// The smallest jump the stepper makes: two of the smallest common plates.
    pub fn step(self) -> f64 {
        match self {
            Unit::Kg => 2.5,
            Unit::Lb => 5.0,
        }
    }

    pub fn bar(self) -> f64 {
        match self {
            Unit::Kg => 20.0,
            Unit::Lb => 45.0,
        }
    }

    pub fn parse(s: &str) -> Option<Unit> {
        match s.trim().to_ascii_lowercase().as_str() {
            "kg" | "kgs" | "kilo" | "kilos" | "kilogram" | "kilograms" => Some(Unit::Kg),
            "lb" | "lbs" | "pound" | "pounds" => Some(Unit::Lb),
            _ => None,
        }
    }
}

/// A weight in whole grams.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct Weight(pub i64);

impl Weight {
    pub fn from_unit(value: f64, unit: Unit) -> Weight {
        let grams = match unit {
            Unit::Kg => value * 1000.0,
            Unit::Lb => value * GRAMS_PER_LB,
        };
        Weight(grams.round() as i64)
    }

    pub fn in_unit(self, unit: Unit) -> f64 {
        match unit {
            Unit::Kg => self.0 as f64 / 1000.0,
            Unit::Lb => self.0 as f64 / GRAMS_PER_LB,
        }
    }

    /// The value shown to the user: kilograms to the nearest quarter (1.25 kg plates
    /// and micro plates), pounds to the nearest half.
    pub fn display_value(self, unit: Unit) -> f64 {
        match unit {
            Unit::Kg => (self.in_unit(unit) * 4.0).round() / 4.0,
            Unit::Lb => (self.in_unit(unit) * 2.0).round() / 2.0,
        }
    }

    /// The nearest weight that can be loaded in steps of `step` (e.g. 2.5 kg, 5 lb).
    pub fn loadable(self, unit: Unit, step: f64) -> f64 {
        if step <= 0.0 {
            return self.display_value(unit);
        }
        ((self.in_unit(unit) / step).round() * step * 100.0).round() / 100.0
    }

    /// "80", "47.5", "82.25". No trailing zeros.
    pub fn display(self, unit: Unit) -> String {
        format_number(self.display_value(unit))
    }

    pub fn display_with_unit(self, unit: Unit) -> String {
        format!("{} {}", self.display(unit), unit.label())
    }
}

pub fn format_number(v: f64) -> String {
    let rounded = (v * 100.0).round() / 100.0;
    if rounded.fract() == 0.0 {
        format!("{}", rounded as i64)
    } else {
        let s = format!("{rounded:.2}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// "6,838" with a thousands separator, as used on summary screens.
pub fn format_thousands(n: i64) -> String {
    let neg = n < 0;
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    if neg { format!("-{out}") } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kg_round_trip_is_exact() {
        let w = Weight::from_unit(47.5, Unit::Kg);
        assert_eq!(w, Weight(47_500));
        assert_eq!(w.display(Unit::Kg), "47.5");
    }

    #[test]
    fn lb_round_trip_displays_the_entered_number() {
        for lb in [45.0, 135.0, 137.5, 225.0, 2.5] {
            let w = Weight::from_unit(lb, Unit::Lb);
            assert_eq!(w.display_value(Unit::Lb), lb, "{lb} lb came back as {}", w.display(Unit::Lb));
        }
    }

    #[test]
    fn kilos_shown_in_pounds_round_sensibly() {
        let w = Weight::from_unit(80.0, Unit::Kg);
        assert_eq!(w.display_value(Unit::Lb), 176.5);
        assert_eq!(w.loadable(Unit::Lb, 5.0), 175.0);
        assert_eq!(Weight::from_unit(82.5, Unit::Kg).loadable(Unit::Kg, 2.5), 82.5);
    }

    #[test]
    fn numbers_format_without_trailing_zeros() {
        assert_eq!(format_number(80.0), "80");
        assert_eq!(format_number(82.25), "82.25");
        assert_eq!(format_number(93.30), "93.3");
        assert_eq!(format_thousands(6838), "6,838");
        assert_eq!(format_thousands(1234567), "1,234,567");
        assert_eq!(format_thousands(999), "999");
    }
}
