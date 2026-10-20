use anyhow::Result;

use crate::github;
use crate::store::Store;

pub fn ingest(store: &Store, items: &[github::Item]) -> Result<usize> {
    Ok(ingest_report(store, items)?.added)
}

pub(super) fn ingest_report(store: &Store, items: &[github::Item]) -> Result<super::BatchCounts> {
    super::ingest_candidates(
        store,
        "github",
        items
            .iter()
            .filter(|item| item.open)
            .map(|item| (item.title.as_str(), item.source_ref.as_str())),
    )
}
