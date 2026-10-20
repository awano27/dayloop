use anyhow::Result;

use crate::devops;
use crate::store::Store;

pub fn ingest(store: &Store, items: &[devops::Item]) -> Result<usize> {
    Ok(ingest_report(store, items)?.added)
}

pub(super) fn ingest_report(store: &Store, items: &[devops::Item]) -> Result<super::BatchCounts> {
    super::ingest_candidates(
        store,
        "ticket",
        items
            .iter()
            .filter(|item| item.open)
            .map(|item| (item.title.as_str(), item.source_ref.as_str())),
    )
}
