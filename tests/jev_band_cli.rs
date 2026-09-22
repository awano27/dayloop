#[test]
fn dry_eval_prints_empty_counts_and_leaves_config() {
    let text = std::fs::read_to_string("fixtures/jev-band-20.json").unwrap();
    let rows = dayloop::jev_eval::load_sheet(&text).unwrap();
    assert_eq!(rows.len(), 20);
    assert!(rows.iter().all(|r| r.choice.is_empty()));
    assert_eq!(dayloop::jev_eval::commit_floor(&rows), None);
}

#[test]
fn sheet_loader_never_returns_a_ledger_write() {
    let text = std::fs::read_to_string("fixtures/jev-band-20.json").unwrap();
    let rows = dayloop::jev_eval::load_sheet(&text).unwrap();
    assert!(dayloop::jev_eval::commit_floor(&rows).is_none());
}
