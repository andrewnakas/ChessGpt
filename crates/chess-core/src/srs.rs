//! Spaced repetition for puzzles (SM-2, simplified): solved puzzles come back
//! after 1, 3, then interval × ease days; a miss brings a puzzle back in ten
//! minutes and makes it come back sooner from then on.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Card {
    pub interval_days: f64,
    pub ease: f64,
    /// Solves in a row.
    pub reps: u32,
    pub lapses: u32,
}

impl Default for Card {
    fn default() -> Card {
        Card { interval_days: 0.0, ease: 2.5, reps: 0, lapses: 0 }
    }
}

/// Ten minutes, in days.
pub const RETRY_DAYS: f64 = 10.0 / (24.0 * 60.0);

pub fn review(c: Card, solved: bool) -> Card {
    if solved {
        let reps = c.reps + 1;
        let interval_days = match reps {
            1 => 1.0,
            2 => 3.0,
            _ => (c.interval_days * c.ease).min(365.0),
        };
        Card { interval_days, ease: (c.ease + 0.1).min(3.0), reps, lapses: c.lapses }
    } else {
        Card { interval_days: RETRY_DAYS, ease: (c.ease - 0.2).max(1.3), reps: 0, lapses: c.lapses + 1 }
    }
}

/// When the card is due, given the review time (ms since the epoch).
pub fn due_ms(c: &Card, now_ms: i64) -> i64 {
    now_ms + (c.interval_days * 86_400_000.0) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intervals_grow_and_reset() {
        let c = review(Card::default(), true);
        assert_eq!((c.reps, c.interval_days), (1, 1.0));
        let c = review(c, true);
        assert_eq!(c.interval_days, 3.0);
        let c = review(c, true);
        assert!((c.interval_days - 3.0 * 2.7).abs() < 1e-9, "{c:?}");
        let miss = review(c, false);
        assert_eq!((miss.reps, miss.lapses), (0, 1));
        assert_eq!(miss.interval_days, RETRY_DAYS);
        assert!((miss.ease - 2.6).abs() < 1e-9, "{miss:?}");
        assert_eq!(due_ms(&review(Card::default(), true), 0), 86_400_000);
    }
}
