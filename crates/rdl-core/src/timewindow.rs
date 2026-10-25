//! Weekly time windows, as scheduled speed limits and queue schedules use
//! them: some days of the week, from `HH:MM` to `HH:MM`. A window whose end
//! is not after its start runs overnight into the next day and belongs to
//! the day it starts on (`00:00`–`00:00` is that whole day).

use chrono::{Datelike, NaiveDateTime, Timelike};

/// Minutes since midnight of `HH:MM`, or `None` if it isn't one.
pub fn minutes(time: &str) -> Option<u32> {
    let time = time.trim();
    let (h, m) = time.split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60 && time.len() == 5).then_some(h * 60 + m)
}

/// Whether the window (`days`: 0 is Monday) covers `now`.
pub fn covers(days: &[u8], from: &str, to: &str, now: NaiveDateTime) -> bool {
    let (Some(from), Some(to)) = (minutes(from), minutes(to)) else {
        return false;
    };
    let day = now.weekday().num_days_from_monday() as u8;
    let yesterday = (day + 6) % 7;
    let at = now.hour() * 60 + now.minute();
    if from < to {
        days.contains(&day) && (from..to).contains(&at)
    } else {
        (days.contains(&day) && at >= from) || (days.contains(&yesterday) && at < to)
    }
}

/// Sorted, without repeats; `None` if empty or a day is out of range.
pub fn tidy_days(mut days: Vec<u8>) -> Option<Vec<u8>> {
    days.sort_unstable();
    days.dedup();
    (!days.is_empty() && days.iter().all(|d| *d <= 6)).then_some(days)
}
