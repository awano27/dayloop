use anyhow::{anyhow, Result};
use chrono::{Datelike, Duration, Local, NaiveDate, Weekday};

pub fn now() -> String {
    Local::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string()
}

pub fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

pub fn parse_date(s: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|_| anyhow!("日付は YYYY-MM-DD 形式で指定してください: {s}"))
}

fn fmt(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

pub fn resolve_date(opt: Option<&str>) -> Result<String> {
    match opt {
        Some(s) => {
            parse_date(s)?;
            Ok(s.to_string())
        }
        None => Ok(today()),
    }
}

/// Next weekday after `date` (Sat/Sun skipped).
pub fn next_workday(date: &str) -> Result<String> {
    let mut d = parse_date(date)? + Duration::days(1);
    while matches!(d.weekday(), Weekday::Sat | Weekday::Sun) {
        d += Duration::days(1);
    }
    Ok(fmt(d))
}

/// Monday..Sunday containing `date`.
pub fn week_range(date: &str) -> Result<(String, String)> {
    let d = parse_date(date)?;
    let mon = d - Duration::days(d.weekday().num_days_from_monday() as i64);
    let sun = mon + Duration::days(6);
    Ok((fmt(mon), fmt(sun)))
}

/// Whole days since an RFC3339-ish timestamp (uses its date part only).
pub fn days_since(ts: &str) -> i64 {
    let date_part = ts.get(..10).unwrap_or(ts);
    match parse_date(date_part) {
        Ok(d) => (Local::now().date_naive() - d).num_days(),
        Err(_) => 0,
    }
}

/// Last 8 chars of a ULID: the random part, so tasks created in the same second still differ.
pub fn short(id: &str) -> &str {
    &id[id.len().saturating_sub(8)..]
}

pub fn hhmm(ts: &str) -> &str {
    ts.get(11..16).unwrap_or(ts)
}
