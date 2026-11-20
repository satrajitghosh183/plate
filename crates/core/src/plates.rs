//! Which plates go on each side of the bar. Colours follow competition plates.

use crate::units::Unit;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plate {
    pub weight: f64,
    /// Base, highlight and shadow colours for the drawn plate.
    pub colors: [&'static str; 3],
    /// Radius and thickness in the 390-wide phone drawing.
    pub radius: u32,
    pub thickness: u32,
}

const STEEL: [&str; 3] = ["#8f8f95", "#d0d0d6", "#3f3f44"];
const RED: [&str; 3] = ["#c8443a", "#ee7d73", "#651a15"];
const BLUE: [&str; 3] = ["#3566bd", "#6f9bea", "#172f60"];
const YELLOW: [&str; 3] = ["#d9a92e", "#f6d57a", "#6e540d"];
const GREEN: [&str; 3] = ["#34935f", "#66c592", "#12452a"];
const WHITE: [&str; 3] = ["#e6e3dc", "#ffffff", "#827f78"];

pub fn kit(unit: Unit) -> Vec<Plate> {
    let p = |weight, colors, radius, thickness| Plate { weight, colors, radius, thickness };
    match unit {
        Unit::Kg => vec![
            p(25.0, RED, 104, 30),
            p(20.0, BLUE, 104, 26),
            p(15.0, YELLOW, 94, 22),
            p(10.0, GREEN, 80, 18),
            p(5.0, WHITE, 60, 12),
            p(2.5, STEEL, 46, 9),
            p(1.25, STEEL, 40, 7),
        ],
        Unit::Lb => vec![
            p(45.0, BLUE, 104, 28),
            p(35.0, YELLOW, 96, 24),
            p(25.0, GREEN, 84, 20),
            p(10.0, WHITE, 62, 13),
            p(5.0, STEEL, 48, 10),
            p(2.5, STEEL, 40, 8),
        ],
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Loading {
    pub per_side: Vec<Plate>,
    /// Weight that cannot be made with the kit (per side), shown as a warning.
    pub leftover_per_side: f64,
}

impl Loading {
    pub fn describe(&self, unit: Unit) -> String {
        let bar = crate::units::format_number(unit.bar());
        if self.per_side.is_empty() {
            return format!("Empty {bar} {} bar", unit.label());
        }
        let names: Vec<String> = self.per_side.iter().map(|p| crate::units::format_number(p.weight)).collect();
        format!("{bar} {} bar + {} per side", unit.label(), names.join(" + "))
    }
}

/// Greedy loading, heaviest plates first, at most nine plates a side.
pub fn load(total: f64, unit: Unit) -> Loading {
    let mut left = ((total - unit.bar()) / 2.0).max(0.0);
    let mut per_side = Vec::new();
    for p in kit(unit) {
        while left + 1e-9 >= p.weight && per_side.len() < 9 {
            per_side.push(p);
            left -= p.weight;
        }
    }
    Loading { per_side, leftover_per_side: (left * 1000.0).round() / 1000.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(l: &Loading) -> Vec<f64> {
        l.per_side.iter().map(|p| p.weight).collect()
    }

    #[test]
    fn loads_like_the_prototype() {
        assert_eq!(w(&load(80.0, Unit::Kg)), vec![25.0, 5.0]);
        assert_eq!(w(&load(82.5, Unit::Kg)), vec![25.0, 5.0, 1.25]);
        assert_eq!(w(&load(90.0, Unit::Kg)), vec![25.0, 10.0]);
        assert_eq!(w(&load(47.5, Unit::Kg)), vec![10.0, 2.5, 1.25]);
        assert_eq!(load(80.0, Unit::Kg).describe(Unit::Kg), "20 kg bar + 25 + 5 per side");
        assert_eq!(load(20.0, Unit::Kg).describe(Unit::Kg), "Empty 20 kg bar");
        assert_eq!(w(&load(225.0, Unit::Lb)), vec![45.0, 45.0]);
        assert_eq!(load(81.0, Unit::Kg).leftover_per_side, 0.5);
    }
}
