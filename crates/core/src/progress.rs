//! Estimated one-rep max and totals.

use crate::units::Weight;

/// Epley: weight × (1 + reps / 30). A single is its own one-rep max.
pub fn e1rm(weight: Weight, reps: u32) -> Weight {
    if reps == 0 {
        return Weight(0);
    }
    if reps == 1 {
        return weight;
    }
    Weight((weight.0 as f64 * (1.0 + reps as f64 / 30.0)).round() as i64)
}

/// Best estimated one-rep max among a session's sets.
pub fn best_e1rm<I: IntoIterator<Item = (Weight, u32)>>(sets: I) -> Option<Weight> {
    sets.into_iter().map(|(w, r)| e1rm(w, r)).filter(|w| w.0 > 0).max()
}

/// Total weight moved, in grams.
pub fn volume<I: IntoIterator<Item = (Weight, u32)>>(sets: I) -> i64 {
    sets.into_iter().map(|(w, r)| w.0 * r as i64).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epley() {
        // 80 kg × 5 → 93.33 kg, the figure on the Progress screen.
        assert_eq!(e1rm(Weight(80_000), 5), Weight(93_333));
        assert_eq!(e1rm(Weight(100_000), 1), Weight(100_000));
        assert_eq!(e1rm(Weight(100_000), 0), Weight(0));
        assert_eq!(best_e1rm([(Weight(80_000), 5), (Weight(85_000), 3)]), Some(Weight(93_500)));
        assert_eq!(volume([(Weight(80_000), 5), (Weight(80_000), 5)]), 800_000);
    }
}
