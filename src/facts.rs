//! Date and calendar facts. Jev never compares these.

use chrono::NaiveTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DueRelation {
    Overdue,
    Today,
    Future,
    None,
}

impl DueRelation {
    pub fn as_str(self) -> &'static str {
        match self {
            DueRelation::Overdue => "overdue",
            DueRelation::Today => "today",
            DueRelation::Future => "future",
            DueRelation::None => "none",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    Fits,
    Over,
    Unknown,
}

impl Fit {
    pub fn as_str(self) -> &'static str {
        match self {
            Fit::Fits => "fits",
            Fit::Over => "over",
            Fit::Unknown => "unknown",
        }
    }
}

/// `today` and `due` are `YYYY-MM-DD`.
pub fn due_relation(today: &str, due: Option<&str>) -> DueRelation {
    match due {
        None => DueRelation::None,
        Some(d) if d < today => DueRelation::Overdue,
        Some(d) if d == today => DueRelation::Today,
        Some(_) => DueRelation::Future,
    }
}

pub fn span(start: &str, end: &str) -> Option<(NaiveTime, NaiveTime)> {
    Some((parse_time(start)?, parse_time(end)?))
}

fn parse_time(s: &str) -> Option<NaiveTime> {
    if let Ok(t) = NaiveTime::parse_from_str(s, "%H:%M") {
        return Some(t);
    }
    if s.len() >= 16 {
        return NaiveTime::parse_from_str(&s[11..16], "%H:%M").ok();
    }
    None
}

/// Largest free gap inside 09:00–18:00, after merging the given spans.
/// No events → [`Fit::Unknown`] (an empty calendar is not zero free minutes).
pub fn fit_estimate(estimate_min: Option<i64>, events: &[(NaiveTime, NaiveTime)]) -> Fit {
    let Some(est) = estimate_min else {
        return Fit::Unknown;
    };
    if events.is_empty() {
        return Fit::Unknown;
    }
    let day_start = NaiveTime::from_hms_opt(9, 0, 0).unwrap();
    let day_end = NaiveTime::from_hms_opt(18, 0, 0).unwrap();
    let mut busy: Vec<(NaiveTime, NaiveTime)> = events
        .iter()
        .filter_map(|(a, b)| clip(*a, *b, day_start, day_end))
        .collect();
    if busy.is_empty() {
        let free = (day_end - day_start).num_minutes();
        return if est <= free { Fit::Fits } else { Fit::Over };
    }
    busy.sort_by_key(|(a, _)| *a);
    let mut merged: Vec<(NaiveTime, NaiveTime)> = Vec::new();
    for (a, b) in busy {
        if let Some(last) = merged.last_mut() {
            if a <= last.1 {
                if b > last.1 {
                    last.1 = b;
                }
                continue;
            }
        }
        merged.push((a, b));
    }
    let mut cursor = day_start;
    let mut largest = 0i64;
    for (a, b) in merged {
        if a > cursor {
            largest = largest.max((a - cursor).num_minutes());
        }
        if b > cursor {
            cursor = b;
        }
    }
    if day_end > cursor {
        largest = largest.max((day_end - cursor).num_minutes());
    }
    if est <= largest {
        Fit::Fits
    } else {
        Fit::Over
    }
}

fn clip(a: NaiveTime, b: NaiveTime, start: NaiveTime, end: NaiveTime) -> Option<(NaiveTime, NaiveTime)> {
    if b <= a || b <= start || a >= end {
        return None;
    }
    let a = if a < start { start } else { a };
    let b = if b > end { end } else { b };
    if b <= a {
        None
    } else {
        Some((a, b))
    }
}
