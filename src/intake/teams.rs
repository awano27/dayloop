use anyhow::{anyhow, Result};
use serde::Deserialize;

use crate::store::Store;

#[derive(Deserialize)]
struct TeamsRow {
    source_ref: String,
    title: String,
}

pub fn ingest(store: &Store, text: &str) -> Result<usize> {
    let rows: Vec<TeamsRow> = serde_json::from_str(text)?;
    let mut n = 0;
    for row in rows {
        if row.title.trim().is_empty() || row.source_ref.trim().is_empty() {
            return Err(anyhow!("Teams の title と source_ref が必要です"));
        }
        if store
            .add_candidate(row.title.trim(), "teams", Some(&row.source_ref))?
            .is_some()
        {
            n += 1;
        }
    }
    Ok(n)
}
