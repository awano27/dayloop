use anyhow::Result;

use crate::devops;
use crate::store::Store;

pub fn ingest(store: &Store, items: &[devops::Item]) -> Result<usize> {
    let mut n = 0;
    for item in items.iter().filter(|item| item.open) {
        if store
            .add_candidate(&item.title, "ticket", Some(&item.source_ref))?
            .is_some()
        {
            n += 1;
        }
    }
    Ok(n)
}
