//! The toast: one line that says what just changed somewhere the reader was not looking.
//!
//! It is drawn in the bottom right of the workpanel and it goes away on its own (interface
//! spec 8.1). Stay awake is its first sender; M4's Notifier is the one it was designed for,
//! and it draws into this same surface rather than inventing another.
//!
//! A toast never takes focus and never takes a key. Anything it says must also be readable
//! somewhere that stays, which for stay awake is the dot at the right end (design principle
//! 2, decision 0029).

use chrono::{DateTime, Local};
use std::time::Duration;

/// How long a toast stands. Interface spec 8.1. A constant rather than a setting until M4
/// brings `[notifications.toast]` and the rest of the table with it.
pub const LIFE: Duration = Duration::from_secs(6);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toast {
    /// What it says, first line first. Each line is wrapped to the box when it is drawn, so
    /// a sender writes sentences and not columns.
    pub lines: Vec<String>,
    /// When it goes. A time rather than a countdown, so a slow tick shows it for its six
    /// seconds and not for six ticks.
    pub until: DateTime<Local>,
}

impl Toast {
    pub fn new(text: impl Into<String>, now: DateTime<Local>) -> Toast {
        Toast {
            lines: vec![text.into()],
            until: now + chrono::Duration::from_std(LIFE).expect("six seconds"),
        }
    }

    /// A second line, for what the reader needs beyond the headline: the reason something
    /// they asked for did not happen, and what to run about it.
    pub fn and(mut self, line: impl Into<String>) -> Toast {
        self.lines.push(line.into());
        self
    }

    pub fn expired(&self, now: DateTime<Local>) -> bool {
        now >= self.until
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FixedClock;

    #[test]
    fn a_toast_stands_for_its_life_and_not_a_second_longer() {
        let now = FixedClock::at("2026-09-10T14:32:00").0;
        let t = Toast::new("Stay awake turned on", now);
        assert_eq!(t.lines, vec!["Stay awake turned on".to_string()]);
        assert!(!t.expired(now));
        assert!(!t.expired(now + chrono::Duration::seconds(5)));
        assert!(t.expired(now + chrono::Duration::seconds(6)));
    }
}
