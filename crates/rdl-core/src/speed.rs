//! Which speed limit applies when: the status bar's switch, or a scheduled
//! rule ("weekdays 08:00–18:00, 2 MB/s").
//!
//! The switch wins while it's on. Otherwise every rule whose window covers
//! the moment applies, and the strictest one counts. A window whose end is
//! not after its start runs overnight into the next day (22:00–06:00), and
//! belongs to the day it starts on.

use chrono::{Datelike, NaiveDateTime, Timelike};
use serde::{Deserialize, Serialize};

use crate::Result;
use crate::ratelimit::MIN_RATE;

/// Most rules a schedule may have.
pub const MAX_RULES: usize = 20;

/// A window of the week with its own limit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SpeedRule {
    /// Days the window starts on: 0 is Monday, 6 is Sunday.
    pub days: Vec<u8>,
    /// Start, `HH:MM`.
    pub from: String,
    /// End, `HH:MM`; not after `from` means the next day.
    pub to: String,
    /// Bytes per second.
    pub limit: u64,
}

/// What limits the speed right now, for the status bar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ActiveLimit {
    /// Bytes per second; `None` is no limit.
    pub limit: Option<u64>,
    pub source: LimitSource,
    /// For a scheduled limit, when its window ends (`HH:MM`).
    pub until: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum LimitSource {
    None,
    /// The switch is on.
    Manual,
    Schedule,
}

/// Minutes since midnight of `HH:MM`.
fn minutes(time: &str) -> Option<u32> {
    let (h, m) = time.trim().split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60 && time.trim().len() == 5).then_some(h * 60 + m)
}

impl SpeedRule {
    /// Whether the rule's window covers `now`.
    pub fn covers(&self, now: NaiveDateTime) -> bool {
        let (Some(from), Some(to)) = (minutes(&self.from), minutes(&self.to)) else {
            return false;
        };
        let day = now.weekday().num_days_from_monday() as u8;
        let yesterday = (day + 6) % 7;
        let at = now.hour() * 60 + now.minute();
        if from < to {
            self.days.contains(&day) && (from..to).contains(&at)
        } else {
            (self.days.contains(&day) && at >= from) || (self.days.contains(&yesterday) && at < to)
        }
    }

    /// Check what the UI sent and tidy it (days sorted, without repeats).
    pub fn validated(mut self) -> Result<Self> {
        let invalid = |m: String| Err(crate::Error::InvalidInput(m));
        if minutes(&self.from).is_none() || minutes(&self.to).is_none() {
            return invalid(format!(
                "\"{}–{}\" isn't a time window like 08:00–18:00.",
                self.from, self.to
            ));
        }
        self.days.sort_unstable();
        self.days.dedup();
        if self.days.is_empty() || self.days.iter().any(|d| *d > 6) {
            return invalid("Each scheduled limit needs at least one day.".into());
        }
        if self.limit < MIN_RATE {
            return invalid(format!(
                "A speed limit must be at least {} KB/s.",
                MIN_RATE / 1024
            ));
        }
        self.from = self.from.trim().to_owned();
        self.to = self.to.trim().to_owned();
        Ok(self)
    }
}

/// The limit for `now`: the switch's, else the strictest scheduled one.
pub fn active_limit(
    switch_on: bool,
    limit: u64,
    schedule: &[SpeedRule],
    now: NaiveDateTime,
) -> ActiveLimit {
    if switch_on {
        return ActiveLimit {
            limit: Some(limit),
            source: LimitSource::Manual,
            until: None,
        };
    }
    match schedule
        .iter()
        .filter(|r| r.covers(now))
        .min_by_key(|r| r.limit)
    {
        Some(rule) => ActiveLimit {
            limit: Some(rule.limit),
            source: LimitSource::Schedule,
            until: Some(rule.to.clone()),
        },
        None => ActiveLimit {
            limit: None,
            source: LimitSource::None,
            until: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;

    /// 2026-10-26 is a Monday.
    fn at(day: u32, hm: &str) -> NaiveDateTime {
        let (h, m) = hm.split_once(':').unwrap();
        (NaiveDate::from_ymd_opt(2026, 10, 26).unwrap() + chrono::Days::new(day.into()))
            .and_hms_opt(h.parse().unwrap(), m.parse().unwrap(), 0)
            .unwrap()
    }

    fn rule(days: &[u8], from: &str, to: &str, limit: u64) -> SpeedRule {
        SpeedRule {
            days: days.to_vec(),
            from: from.into(),
            to: to.into(),
            limit,
        }
    }

    #[test]
    fn day_windows() {
        let weekdays = rule(&[0, 1, 2, 3, 4], "08:00", "18:00", 2048);
        assert!(weekdays.covers(at(0, "08:00")));
        assert!(weekdays.covers(at(4, "17:59")));
        assert!(!weekdays.covers(at(0, "18:00")), "the end is outside");
        assert!(!weekdays.covers(at(0, "07:59")));
        assert!(!weekdays.covers(at(5, "12:00")), "Saturday");
    }

    #[test]
    fn overnight_windows_belong_to_the_day_they_start() {
        let friday_night = rule(&[4], "22:00", "06:00", 2048);
        assert!(friday_night.covers(at(4, "23:30")));
        assert!(friday_night.covers(at(5, "05:59")), "Saturday morning");
        assert!(!friday_night.covers(at(5, "06:00")));
        assert!(
            !friday_night.covers(at(4, "05:00")),
            "Friday morning is Thursday's"
        );
        assert!(!friday_night.covers(at(5, "23:00")));
        let sunday_night = rule(&[6], "23:00", "01:00", 2048);
        assert!(sunday_night.covers(at(7, "00:30")), "into Monday");
        let all_day = rule(&[2], "00:00", "00:00", 2048);
        assert!(all_day.covers(at(2, "00:00")) && all_day.covers(at(2, "23:59")));
        assert!(!all_day.covers(at(3, "00:00")));
    }

    #[test]
    fn the_switch_wins_then_the_strictest_rule() {
        let schedule = [
            rule(&[0], "08:00", "18:00", 4096),
            rule(&[0], "12:00", "13:00", 2048),
        ];
        let lunch = at(0, "12:30");
        assert_eq!(
            active_limit(true, 9999, &schedule, lunch),
            ActiveLimit {
                limit: Some(9999),
                source: LimitSource::Manual,
                until: None
            }
        );
        assert_eq!(
            active_limit(false, 9999, &schedule, lunch),
            ActiveLimit {
                limit: Some(2048),
                source: LimitSource::Schedule,
                until: Some("13:00".into())
            }
        );
        assert_eq!(
            active_limit(false, 9999, &schedule, at(0, "09:00")).limit,
            Some(4096)
        );
        assert_eq!(
            active_limit(false, 9999, &schedule, at(0, "19:00")).source,
            LimitSource::None
        );
    }

    #[test]
    fn rules_are_checked() {
        let ok = rule(&[3, 1, 1], " 08:00", "18:00 ", 4096)
            .validated()
            .unwrap();
        assert_eq!(ok.days, [1, 3]);
        assert_eq!((ok.from.as_str(), ok.to.as_str()), ("08:00", "18:00"));
        for bad in [
            rule(&[], "08:00", "18:00", 4096),
            rule(&[7], "08:00", "18:00", 4096),
            rule(&[0], "8:00", "18:00", 4096),
            rule(&[0], "24:00", "18:00", 4096),
            rule(&[0], "08:00", "18:60", 4096),
            rule(&[0], "08:00", "18:00", 10),
        ] {
            assert!(bad.clone().validated().is_err(), "{bad:?}");
        }
    }
}
