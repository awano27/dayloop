//! External evidence. A task becomes done only when a source says it is closed.

use std::collections::BTreeMap;

use anyhow::{anyhow, Result};
use serde::Deserialize;

use crate::model::State;
use crate::store::Store;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sight {
    Closed,
    Open,
}

#[derive(Deserialize)]
struct Row {
    source_ref: String,
    state: String,
}

pub fn parse_fixture(text: &str) -> Result<BTreeMap<String, Sight>> {
    let rows: Vec<Row> = serde_json::from_str(text)?;
    let mut map = BTreeMap::new();
    for row in rows {
        let sight = match row.state.as_str() {
            "closed" | "merged" => Sight::Closed,
            "open" => Sight::Open,
            other => return Err(anyhow!("未知の観測状態: {other}")),
        };
        map.insert(row.source_ref, sight);
    }
    Ok(map)
}

pub fn lookup(map: &BTreeMap<String, Sight>, source_ref: Option<&str>) -> Option<Sight> {
    source_ref.and_then(|key| map.get(key).copied())
}

pub fn load_map(cfg: &crate::config::ObserveConfig) -> Result<BTreeMap<String, Sight>> {
    if cfg.fixture.trim().is_empty() {
        return Ok(BTreeMap::new());
    }
    let text = std::fs::read_to_string(&cfg.fixture)?;
    parse_fixture(&text)
}

pub fn apply(store: &Store, date: &str, map: &BTreeMap<String, Sight>) -> Result<usize> {
    let mut n = 0;
    for t in store.open_tasks_for_day(date)? {
        if lookup(map, t.source_ref.as_deref()) != Some(Sight::Closed) {
            continue;
        }
        let key = t.source_ref.clone().unwrap_or_default();
        let marker = format!("observe:{key}");
        store.transition_silent(&t.id, State::Done, None, Some(&marker))?;
        store.mark_observed(&t.id, &key)?;
        n += 1;
    }
    Ok(n)
}

pub fn meeting_ended(source_ref: &str, events: &[crate::model::Event], now_hhmm: &str) -> bool {
    let Some(rest) = source_ref.strip_prefix("outlook:cal:") else {
        return false;
    };
    let Some(id) = rest.strip_suffix(":prep") else {
        return false;
    };
    let Some((_, now)) = crate::facts::span("00:00", now_hhmm) else {
        return false;
    };
    events.iter().any(|event| {
        if event.entry_id != id {
            return false;
        }
        match crate::facts::span("00:00", &event.end) {
            Some((_, end)) => end <= now,
            None => false,
        }
    })
}

pub fn apply_day(
    store: &Store,
    date: &str,
    map: &BTreeMap<String, Sight>,
    now_hhmm: &str,
) -> Result<usize> {
    let mut n = apply(store, date, map)?;
    let events = store.events_for_day(date)?;
    for t in store.open_tasks_for_day(date)? {
        let Some(source_ref) = t.source_ref.as_deref() else {
            continue;
        };
        if !meeting_ended(source_ref, &events, now_hhmm) {
            continue;
        }
        let marker = format!("observe:{source_ref}");
        store.transition_silent(&t.id, State::Done, None, Some(&marker))?;
        store.mark_observed(&t.id, source_ref)?;
        n += 1;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_and_merged_are_closed() {
        let text = r#"[
            {"source_ref":"ticket:ABC-1","state":"closed"},
            {"source_ref":"github:pr:dayloop#7","state":"merged"},
            {"source_ref":"ticket:ABC-2","state":"open"}
        ]"#;
        let map = parse_fixture(text).unwrap();
        assert_eq!(map.get("ticket:ABC-1"), Some(&Sight::Closed));
        assert_eq!(map.get("github:pr:dayloop#7"), Some(&Sight::Closed));
        assert_eq!(map.get("ticket:ABC-2"), Some(&Sight::Open));
    }

    #[test]
    fn unknown_state_is_rejected() {
        let text = r#"[{"source_ref":"ticket:ABC-3","state":"done"}]"#;
        assert!(parse_fixture(text).is_err());
    }
}
