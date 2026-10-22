// Written by Kone Tshivhinda & Claude
//! Counting outcomes without floating point.
//!
//! A score is "how many out of how many". [`Ratio`] keeps both numbers, so a
//! gate compares them exactly (`97.0%` means `num * 10000 >= den * 9700`) and
//! a report can print both. Percentages are only ever *shown*, rounded down to
//! a hundredth of a percent, never used to decide anything.

use serde::Serialize;
use std::fmt;

/// `num` out of `den`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Ratio {
    pub num: u64,
    pub den: u64,
}

impl Ratio {
    pub const fn new(num: u64, den: u64) -> Ratio {
        Ratio { num, den }
    }

    /// The ratio in basis points (hundredths of a percent), rounded down, or
    /// `None` when there is nothing to measure (`den` is zero).
    pub fn basis_points(self) -> Option<u32> {
        if self.den == 0 {
            return None;
        }
        // 128-bit arithmetic: `num * 10_000` cannot overflow.
        let bp = u128::from(self.num) * 10_000 / u128::from(self.den);
        Some(u32::try_from(bp).unwrap_or(u32::MAX))
    }

    /// Whether the ratio is at least `min_bp` basis points, compared exactly
    /// (no rounding). With nothing to measure it is vacuously true; a gate
    /// reports an empty set of cases separately.
    pub fn at_least(self, min_bp: u32) -> bool {
        u128::from(self.num) * 10_000 >= u128::from(self.den) * u128::from(min_bp)
    }
}

impl fmt::Display for Ratio {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.basis_points() {
            Some(bp) => write!(f, "{}.{:02}% ({}/{})", bp / 100, bp % 100, self.num, self.den),
            None => write!(f, "n/a (0/0)"),
        }
    }
}

/// What happened to one field. "Flagged" means the system said a person must
/// look (see [`crate::score`]); an error that is not flagged is *silent*.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Correct,
    /// A value that differs from the truth.
    WrongFlagged,
    WrongSilent,
    /// The truth has a value; the system gave none.
    MissingFlagged,
    MissingSilent,
    /// The system gave a value; the truth has none.
    SpuriousFlagged,
    SpuriousSilent,
}

impl Outcome {
    /// An error nobody was told about.
    pub fn is_silent_error(self) -> bool {
        matches!(
            self,
            Outcome::WrongSilent | Outcome::MissingSilent | Outcome::SpuriousSilent
        )
    }

    pub fn is_correct(self) -> bool {
        self == Outcome::Correct
    }
}

/// How many fields had each outcome.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Tally {
    pub correct: u64,
    pub wrong_flagged: u64,
    pub wrong_silent: u64,
    pub missing_flagged: u64,
    pub missing_silent: u64,
    pub spurious_flagged: u64,
    pub spurious_silent: u64,
}

impl Tally {
    pub fn add(&mut self, outcome: Outcome) {
        let slot = match outcome {
            Outcome::Correct => &mut self.correct,
            Outcome::WrongFlagged => &mut self.wrong_flagged,
            Outcome::WrongSilent => &mut self.wrong_silent,
            Outcome::MissingFlagged => &mut self.missing_flagged,
            Outcome::MissingSilent => &mut self.missing_silent,
            Outcome::SpuriousFlagged => &mut self.spurious_flagged,
            Outcome::SpuriousSilent => &mut self.spurious_silent,
        };
        *slot += 1;
    }

    pub fn merge(&mut self, other: &Tally) {
        self.correct += other.correct;
        self.wrong_flagged += other.wrong_flagged;
        self.wrong_silent += other.wrong_silent;
        self.missing_flagged += other.missing_flagged;
        self.missing_silent += other.missing_silent;
        self.spurious_flagged += other.spurious_flagged;
        self.spurious_silent += other.spurious_silent;
    }

    pub fn total(&self) -> u64 {
        self.correct + self.errors()
    }

    pub fn errors(&self) -> u64 {
        self.wrong_flagged
            + self.wrong_silent
            + self.missing_flagged
            + self.missing_silent
            + self.spurious_flagged
            + self.spurious_silent
    }

    pub fn silent_errors(&self) -> u64 {
        self.wrong_silent + self.missing_silent + self.spurious_silent
    }

    /// Correct fields out of all fields, whether or not the errors were
    /// flagged: the accuracy before any human review.
    pub fn accuracy(&self) -> Ratio {
        Ratio::new(self.correct, self.total())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basis_points_round_down_and_zero_denominators_have_none() {
        assert_eq!(Ratio::new(97, 100).basis_points(), Some(9_700));
        assert_eq!(Ratio::new(1, 3).basis_points(), Some(3_333));
        assert_eq!(Ratio::new(2, 3).basis_points(), Some(6_666));
        assert_eq!(Ratio::new(0, 5).basis_points(), Some(0));
        assert_eq!(Ratio::new(5, 5).basis_points(), Some(10_000));
        assert_eq!(Ratio::new(0, 0).basis_points(), None);
    }

    #[test]
    fn huge_counts_do_not_overflow() {
        assert_eq!(Ratio::new(u64::MAX, u64::MAX).basis_points(), Some(10_000));
        assert!(Ratio::new(u64::MAX, u64::MAX).at_least(10_000));
        assert!(!Ratio::new(u64::MAX - 1, u64::MAX).at_least(10_000));
    }

    #[test]
    fn at_least_is_exact_at_the_boundary() {
        // 97 of 100 is exactly 97.00%.
        assert!(Ratio::new(97, 100).at_least(9_700));
        assert!(!Ratio::new(96, 100).at_least(9_700));
        // 2 of 3 is 66.666...%: not at least 66.67%, though it prints as 66.66%.
        assert!(!Ratio::new(2, 3).at_least(6_667));
        assert!(Ratio::new(2, 3).at_least(6_666));
        // Nothing to measure is vacuously fine.
        assert!(Ratio::new(0, 0).at_least(10_000));
        assert!(Ratio::new(0, 4).at_least(0));
        assert!(!Ratio::new(0, 4).at_least(1));
    }

    #[test]
    fn ratios_print_both_numbers() {
        assert_eq!(Ratio::new(39, 40).to_string(), "97.50% (39/40)");
        assert_eq!(Ratio::new(1, 3).to_string(), "33.33% (1/3)");
        assert_eq!(Ratio::new(5, 5).to_string(), "100.00% (5/5)");
        assert_eq!(Ratio::new(0, 0).to_string(), "n/a (0/0)");
        assert_eq!(Ratio::new(1, 20).to_string(), "5.00% (1/20)");
    }

    #[test]
    fn only_unflagged_errors_are_silent() {
        use Outcome::*;
        for (outcome, silent) in [
            (Correct, false),
            (WrongFlagged, false),
            (WrongSilent, true),
            (MissingFlagged, false),
            (MissingSilent, true),
            (SpuriousFlagged, false),
            (SpuriousSilent, true),
        ] {
            assert_eq!(outcome.is_silent_error(), silent, "{outcome:?}");
            assert_eq!(outcome.is_correct(), outcome == Correct);
        }
    }

    #[test]
    fn a_tally_counts_each_outcome_in_its_own_slot() {
        use Outcome::*;
        let mut tally = Tally::default();
        for outcome in [
            Correct,
            Correct,
            Correct,
            WrongFlagged,
            WrongSilent,
            WrongSilent,
            MissingFlagged,
            MissingSilent,
            SpuriousFlagged,
            SpuriousSilent,
        ] {
            tally.add(outcome);
        }
        assert_eq!(
            tally,
            Tally {
                correct: 3,
                wrong_flagged: 1,
                wrong_silent: 2,
                missing_flagged: 1,
                missing_silent: 1,
                spurious_flagged: 1,
                spurious_silent: 1,
            }
        );
        assert_eq!(tally.total(), 10);
        assert_eq!(tally.errors(), 7);
        assert_eq!(tally.silent_errors(), 4);
        assert_eq!(tally.accuracy(), Ratio::new(3, 10));
    }

    #[test]
    fn merging_adds_every_slot() {
        let mut a = Tally {
            correct: 1,
            wrong_flagged: 2,
            wrong_silent: 3,
            missing_flagged: 4,
            missing_silent: 5,
            spurious_flagged: 6,
            spurious_silent: 7,
        };
        let b = Tally {
            correct: 10,
            wrong_flagged: 20,
            wrong_silent: 30,
            missing_flagged: 40,
            missing_silent: 50,
            spurious_flagged: 60,
            spurious_silent: 70,
        };
        a.merge(&b);
        assert_eq!(
            a,
            Tally {
                correct: 11,
                wrong_flagged: 22,
                wrong_silent: 33,
                missing_flagged: 44,
                missing_silent: 55,
                spurious_flagged: 66,
                spurious_silent: 77
            }
        );
    }

    #[test]
    fn an_empty_tally_has_no_accuracy() {
        assert_eq!(Tally::default().accuracy().basis_points(), None);
    }
}
