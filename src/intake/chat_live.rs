use anyhow::Result;

use crate::chat;
use crate::store::Store;

pub fn ingest(store: &Store, items: &[chat::Item]) -> Result<usize> {
    let mut n = 0;
    for item in items {
        if store
            .add_candidate(&item.title, "teams", Some(&item.source_ref))?
            .is_some()
        {
            n += 1;
        }
    }
    Ok(n)
}
