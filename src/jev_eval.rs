//! Agreement bands for the 20-subject sheet. This module does not write config.

use anyhow::{bail, Result};
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    Below50,
    From50To70,
    From70To85,
    AtLeast85,
}

pub struct Row {
    pub subject: String,
    pub human: String,
    pub choice: String,
    pub confidence: f64,
}

pub fn band(confidence: f64) -> Band {
    if confidence < 0.5 {
        Band::Below50
    } else if confidence < 0.7 {
        Band::From50To70
    } else if confidence < 0.85 {
        Band::From70To85
    } else {
        Band::AtLeast85
    }
}

pub fn commit_floor(rows: &[Row]) -> Option<f64> {
    for (target, floor) in [
        (Band::From50To70, 0.5),
        (Band::From70To85, 0.7),
        (Band::AtLeast85, 0.85),
    ] {
        let group: Vec<&Row> = rows.iter().filter(|r| band(r.confidence) == target).collect();
        if group.len() >= 5 && group.iter().all(|r| r.human == r.choice) {
            return Some(floor);
        }
    }
    None
}

#[derive(Deserialize)]
struct SheetRow {
    subject: String,
    human: String,
    #[serde(default)]
    choice: String,
    #[serde(default)]
    confidence: f64,
}

pub fn load_sheet(text: &str) -> Result<Vec<Row>> {
    let raw: Vec<SheetRow> = serde_json::from_str(text)?;
    if raw.len() != 20 {
        bail!("帯の表は20件です: {}", raw.len());
    }
    Ok(raw
        .into_iter()
        .map(|r| Row {
            subject: r.subject,
            human: r.human,
            choice: r.choice,
            confidence: r.confidence,
        })
        .collect())
}

pub fn report(rows: &[Row]) -> String {
    let mut out = String::new();
    for (target, label) in [
        (Band::Below50, "0.5 未満"),
        (Band::From50To70, "0.5–0.7"),
        (Band::From70To85, "0.7–0.85"),
        (Band::AtLeast85, "0.85 以上"),
    ] {
        let group: Vec<&Row> = rows
            .iter()
            .filter(|r| band(r.confidence) == target && !r.choice.is_empty())
            .collect();
        let matched = group.iter().filter(|r| r.human == r.choice).count();
        out.push_str(&format!("{label}: 件数 {} / 一致 {matched}\n", group.len()));
    }
    match commit_floor(rows) {
        Some(floor) => out.push_str(&format!("下限候補: {floor}\nこの値は設定へ書かない\n")),
        None => out.push_str("下限はまだ書かない\n"),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(confidence: f64, same: bool) -> Row {
        Row {
            subject: "件名".into(),
            human: "task".into(),
            choice: if same { "task".into() } else { "info".into() },
            confidence,
        }
    }

    #[test]
    fn bands_follow_the_sheet() {
        assert_eq!(band(0.49), Band::Below50);
        assert_eq!(band(0.5), Band::From50To70);
        assert_eq!(band(0.7), Band::From70To85);
        assert_eq!(band(0.85), Band::AtLeast85);
    }

    #[test]
    fn floor_is_the_lowest_band_that_matches_completely() {
        let mut rows = Vec::new();
        for _ in 0..5 {
            rows.push(row(0.9, true));
        }
        for _ in 0..5 {
            rows.push(row(0.6, false));
        }
        assert_eq!(commit_floor(&rows), Some(0.85));
    }

    #[test]
    fn fewer_than_five_matches_is_not_enough() {
        let rows = vec![row(0.9, true), row(0.9, true)];
        assert_eq!(commit_floor(&rows), None);
    }
}
