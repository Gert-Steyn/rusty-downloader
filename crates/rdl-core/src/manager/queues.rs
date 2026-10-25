//! Queues: each has its own limit of downloads at once, an optional weekly
//! window outside which it starts nothing (and stops what runs when the
//! window closes), and something to do when it has finished.
//!
//! A queue counts as finished when at least one of its downloads ended
//! (completed or failed) since it last finished, and none is running,
//! waiting or paused. Pausing everything or removing downloads therefore
//! never sets it off, and failures don't keep an overnight queue from
//! finishing.

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

use super::types::QueueId;
use crate::timewindow;
use crate::{Error, Result};

/// Longest queue name.
const MAX_NAME: usize = 60;

/// When a queue may run. Disabled: always.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct QueueSchedule {
    pub enabled: bool,
    /// Days the window starts on: 0 is Monday, 6 is Sunday.
    pub days: Vec<u8>,
    /// `HH:MM`; a stop not after the start means the next day.
    pub start: String,
    pub stop: String,
}

impl Default for QueueSchedule {
    fn default() -> Self {
        Self {
            enabled: false,
            days: (0..7).collect(),
            start: "01:00".into(),
            stop: "07:00".into(),
        }
    }
}

impl QueueSchedule {
    /// Whether the queue may run at `now`.
    pub fn is_open(&self, now: NaiveDateTime) -> bool {
        !self.enabled || timewindow::covers(&self.days, &self.start, &self.stop, now)
    }

    fn validated(mut self) -> Result<Self> {
        let invalid = |m: &str| Err(Error::InvalidInput(m.into()));
        if timewindow::minutes(&self.start).is_none() || timewindow::minutes(&self.stop).is_none() {
            return invalid("Start and stop must be times like 01:00.");
        }
        match timewindow::tidy_days(self.days) {
            Some(days) => self.days = days,
            None => return invalid("A schedule needs at least one day."),
        }
        self.start = self.start.trim().to_owned();
        self.stop = self.stop.trim().to_owned();
        Ok(self)
    }
}

/// What to do when a queue has finished. Stored as `queues.on_complete_action`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum OnComplete {
    #[default]
    None,
    Sleep,
    Hibernate,
    Shutdown,
    /// Close the app.
    Exit,
}

impl OnComplete {
    pub fn as_str(self) -> &'static str {
        match self {
            OnComplete::None => "none",
            OnComplete::Sleep => "sleep",
            OnComplete::Hibernate => "hibernate",
            OnComplete::Shutdown => "shutdown",
            OnComplete::Exit => "exit",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "sleep" => OnComplete::Sleep,
            "hibernate" => OnComplete::Hibernate,
            "shutdown" => OnComplete::Shutdown,
            "exit" => OnComplete::Exit,
            _ => OnComplete::None,
        }
    }
}

/// A queue as the Queues page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Queue {
    pub id: QueueId,
    pub name: String,
    pub max_active: u32,
    pub schedule: QueueSchedule,
    pub on_complete: OnComplete,
    /// Whether its schedule lets it run right now.
    pub open: bool,
    pub packages: u32,
    pub active: u32,
    pub queued: u32,
    pub paused: u32,
}

/// What the Queues page can change about a queue.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct QueueChanges {
    pub name: String,
    pub max_active: u32,
    pub schedule: QueueSchedule,
    pub on_complete: OnComplete,
}

impl QueueChanges {
    pub fn validated(mut self) -> Result<Self> {
        self.name = self.name.trim().to_owned();
        if self.name.is_empty() || self.name.chars().count() > MAX_NAME {
            return Err(Error::InvalidInput(format!(
                "A queue needs a name of up to {MAX_NAME} characters."
            )));
        }
        if !(1..=crate::settings::MAX_ACTIVE).contains(&self.max_active) {
            return Err(Error::InvalidInput(format!(
                "Downloads at once must be between 1 and {}.",
                crate::settings::MAX_ACTIVE
            )));
        }
        self.schedule = self.schedule.validated()?;
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;

    fn at(hm: &str) -> NaiveDateTime {
        // 2026-10-26 is a Monday.
        let (h, m) = hm.split_once(':').unwrap();
        NaiveDate::from_ymd_opt(2026, 10, 26)
            .unwrap()
            .and_hms_opt(h.parse().unwrap(), m.parse().unwrap(), 0)
            .unwrap()
    }

    #[test]
    fn schedules_open_and_close() {
        let mut s = QueueSchedule::default();
        assert!(s.is_open(at("12:00")), "disabled: always open");
        s.enabled = true;
        assert!(s.is_open(at("01:00")) && s.is_open(at("06:59")));
        assert!(!s.is_open(at("07:00")) && !s.is_open(at("12:00")));
        s.start = "23:00".into();
        s.stop = "06:00".into();
        assert!(s.is_open(at("05:00")), "Sunday night's window");
        assert!(!s.is_open(at("22:00")));
    }

    #[test]
    fn changes_are_checked() {
        let ok = QueueChanges {
            name: "  Night  ".into(),
            max_active: 2,
            schedule: QueueSchedule {
                days: vec![4, 0, 0],
                ..QueueSchedule::default()
            },
            on_complete: OnComplete::Shutdown,
        };
        let tidy = ok.clone().validated().unwrap();
        assert_eq!(tidy.name, "Night");
        assert_eq!(tidy.schedule.days, [0, 4]);
        for bad in [
            QueueChanges {
                name: " ".into(),
                ..ok.clone()
            },
            QueueChanges {
                max_active: 0,
                ..ok.clone()
            },
            QueueChanges {
                schedule: QueueSchedule {
                    stop: "7:00".into(),
                    ..QueueSchedule::default()
                },
                ..ok.clone()
            },
            QueueChanges {
                schedule: QueueSchedule {
                    days: vec![],
                    ..QueueSchedule::default()
                },
                ..ok.clone()
            },
        ] {
            assert!(bad.validated().is_err());
        }
        for action in ["none", "sleep", "hibernate", "shutdown", "exit"] {
            assert_eq!(OnComplete::parse(action).as_str(), action);
        }
    }
}
