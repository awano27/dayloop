use anyhow::Result;

use crate::chat;
use crate::store::Store;

pub fn ingest(store: &Store, items: &[chat::Item]) -> Result<usize> {
    Ok(ingest_report(store, items)?.added)
}

pub(super) fn ingest_report(store: &Store, items: &[chat::Item]) -> Result<super::BatchCounts> {
    super::ingest_candidates(
        store,
        "teams",
        items
            .iter()
            .map(|item| (item.title.as_str(), item.source_ref.as_str())),
    )
}
