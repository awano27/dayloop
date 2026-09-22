use anyhow::{anyhow, Result};
use serde::Deserialize;

use crate::store::Store;

#[derive(Deserialize)]
struct TicketRow {
    source_ref: String,
    title: String,
    state: String,
}

pub fn ingest(store: &Store, text: &str) -> Result<usize> {
    let rows: Vec<TicketRow> = serde_json::from_str(text)?;
    let mut n = 0;
    for row in rows {
        if row.title.trim().is_empty() || row.source_ref.trim().is_empty() {
            return Err(anyhow!("チケットの title と source_ref が必要です"));
        }
        match row.state.as_str() {
            "open" | "closed" => {}
            other => return Err(anyhow!("チケットの state が未知です: {other}")),
        }
        if store
            .add_candidate(row.title.trim(), "ticket", Some(&row.source_ref))?
            .is_some()
        {
            n += 1;
        }
    }
    Ok(n)
}
