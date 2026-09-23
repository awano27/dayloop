//! Stage 3.1–3.3: reason codes, proposals, decision graph, Jev only on unknown nodes.

use std::path::PathBuf;
use std::sync::Mutex;

use dayloop::engine::{self, Question};
use dayloop::facts::{self, DueRelation, Fit};
use dayloop::graph::{self, Situation};
use dayloop::jev::{self, Decider, JevOutcome};
use dayloop::model::State;
use dayloop::reason::ReasonCode;
use dayloop::rituals::{self, Outcome, Ui};
use dayloop::store::Store;
use dayloop::tools;

static LOCK: Mutex<()> = Mutex::new(());

struct Home {
    dir: PathBuf,
    _guard: std::sync::MutexGuard<'static, ()>,
    old: Option<String>,
}

impl Home {
    fn new() -> Self {
        let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let old = std::env::var("DAYLOOP_HOME").ok();
        let dir = std::env::temp_dir().join(format!(
            "dayloop-growth-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        unsafe {
            std::env::set_var("DAYLOOP_HOME", &dir);
        }
        Self {
            dir,
            _guard: guard,
            old,
        }
    }

    fn store(&self) -> Store {
        Store::open().unwrap()
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        unsafe {
            match &self.old {
                Some(v) => std::env::set_var("DAYLOOP_HOME", v),
                None => std::env::remove_var("DAYLOOP_HOME"),
            }
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

struct Scripted {
    choice: String,
    confidence: f64,
    calls: usize,
    down: bool,
}

impl Decider for Scripted {
    fn decide(&mut self, _state: &str, _choices: &[String]) -> JevOutcome {
        self.calls += 1;
        if self.down {
            JevOutcome::Unavailable
        } else {
            JevOutcome::Answer {
                choice: self.choice.clone(),
                confidence: self.confidence,
            }
        }
    }
}

#[test]
fn reason_code_alone_satisfies_invariant_2() {
    let home = Home::new();
    let store = home.store();
    let d = "2026-09-03";
    let a = store.add_task("A", None, None, "manual", None, Some(d)).unwrap();
    let b = store.add_task("B", None, None, "manual", None, Some(d)).unwrap();
    assert!(store.transition(&a.id, State::NotDone, Some(""), None).is_err());
    let a = store
        .transition(&a.id, State::NotDone, Some("no_time"), None)
        .unwrap();
    assert_eq!(a.state_reason.as_deref(), Some(ReasonCode::NoTime.label_ja()));
    store.carry_over(&b.id, "waiting", "2026-09-04", None).unwrap();
    let carried = store.get_task(&b.id).unwrap();
    assert_eq!(carried.state, State::Carried);
    assert_eq!(carried.state_reason.as_deref(), Some("待ち"));
}

#[test]
fn confirm_plan_rejects_while_yesterday_is_open() {
    let home = Home::new();
    let store = home.store();
    store
        .add_task("昨日", None, None, "manual", None, Some("2026-09-02"))
        .unwrap();
    assert!(store.confirm_plan("2026-09-03").is_err());
    let view = engine::plan_view(&store, "2026-09-03").unwrap();
    let kinds: Vec<&str> = view["questions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|q| q["kind"].as_str())
        .collect();
    assert!(!kinds.contains(&"confirm_plan"));

    let t = &store.open_tasks_for_day("2026-09-02").unwrap()[0];
    store.transition(&t.id, State::Done, None, None).unwrap();
    store.close_day("2026-09-02").unwrap().unwrap();
    store.confirm_plan("2026-09-03").unwrap();
    let view = engine::plan_view(&store, "2026-09-03").unwrap();
    let kinds: Vec<&str> = view["questions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|q| q["kind"].as_str())
        .collect();
    assert!(kinds.contains(&"confirm_plan"));
}

#[test]
fn proposal_does_not_close_the_day_until_commit() {
    let home = Home::new();
    let store = home.store();
    let d = "2026-09-03";
    let t = store.add_task("残件", None, None, "manual", None, Some(d)).unwrap();
    store
        .set_proposal(&t.id, State::Done, None, Some(0.9))
        .unwrap();
    assert!(store.close_day(d).unwrap().is_err());
    let committed = store.commit_proposal(&t.id).unwrap();
    assert_eq!(committed.state, State::Done);
    store.close_day(d).unwrap().unwrap();
}

#[test]
fn shelved_candidate_is_not_a_permanent_reject() {
    let home = Home::new();
    let store = home.store();
    let c = store
        .add_candidate("案内", "outlook", Some("outlook:mail:1"))
        .unwrap()
        .unwrap();
    store.shelve_candidate(&c.id).unwrap();
    assert!(store.open_candidates().unwrap().is_empty());
    assert!(store
        .add_candidate("再掲ではない", "outlook", Some("outlook:mail:1"))
        .unwrap()
        .is_none());
    let again = store
        .add_candidate("別件", "outlook", Some("outlook:mail:2"))
        .unwrap()
        .unwrap();
    assert_eq!(again.status, "open");
}

#[test]
fn carried_from_is_visible_on_the_new_task() {
    let home = Home::new();
    let store = home.store();
    let t = store
        .add_task("連鎖", None, None, "manual", None, Some("2026-09-01"))
        .unwrap();
    let n = store.carry_over(&t.id, "no_time", "2026-09-02", None).unwrap();
    assert_eq!(n.carried_from.as_deref(), Some(t.id.as_str()));
}

#[test]
fn same_title_carries_without_a_question_then_revise_drops_the_next() {
    let home = Home::new();
    let store = home.store();
    let first = store
        .add_task("経費精算", None, None, "manual", None, Some("2026-09-01"))
        .unwrap();
    store
        .carry_over(&first.id, "no_time", "2026-09-02", None)
        .unwrap();

    let day2 = &store.open_tasks_for_day("2026-09-02").unwrap()[0].id.clone();
    let applied = graph::apply_known_tasks(&store, "2026-09-02").unwrap();
    assert_eq!(applied, 1);
    assert_eq!(store.get_task(day2).unwrap().state, State::Carried);
    assert!(store.get_task(day2).unwrap().decided_by.unwrap().starts_with("graph:"));

    let day3 = &store.open_tasks_for_day("2026-09-03").unwrap()[0].id.clone();
    store.revise_task(day3, "drop", Some("not_needed")).unwrap();
    assert_eq!(store.get_task(day3).unwrap().state, State::Dropped);

    store
        .add_task("経費精算", None, None, "manual", None, Some("2026-09-04"))
        .unwrap();
    let outcome = rituals::close_ritual(&store, &Ui::new(true), "2026-09-04").unwrap();
    assert!(matches!(outcome, Outcome::Done));
    assert_eq!(
        store.tasks_for_day("2026-09-04").unwrap()[0].state,
        State::Dropped
    );
}

#[test]
fn conflicting_sender_does_not_auto_decide_a_new_subject() {
    let home = Home::new();
    let store = home.store();
    let shelve = Situation::mail("週報", "田中", "2026-09-01");
    let task = Situation::mail("見積", "田中", "2026-09-02");
    graph::record(&store, &shelve, "shelve", None).unwrap();
    graph::record(&store, &task, "today", None).unwrap();
    let fresh = Situation::mail("新しい件名", "田中", "2026-09-08");
    assert!(graph::resolve(&store, &fresh).unwrap().is_none());
}

#[test]
fn unknown_title_asks_jev_once_then_the_edge_applies() {
    let home = Home::new();
    let store = home.store();
    store
        .add_task("新しい調査", None, None, "manual", None, Some("2026-09-01"))
        .unwrap();
    let mut jev = Scripted {
        choice: "done".into(),
        confidence: 0.95,
        calls: 0,
        down: false,
    };
    let proposed = jev::propose_unknown(&store, "2026-09-01", &mut jev, 0.5).unwrap();
    assert_eq!(proposed.len(), 1);
    assert_eq!(jev.calls, 1);
    assert_eq!(store.open_tasks_for_day("2026-09-01").unwrap().len(), 1);

    store
        .revise_task(&proposed[0].id, "done", None)
        .unwrap();
    store
        .add_task("新しい調査", None, None, "manual", None, Some("2026-09-02"))
        .unwrap();
    let proposed = jev::propose_unknown(&store, "2026-09-02", &mut jev, 0.5).unwrap();
    assert!(proposed.is_empty());
    assert_eq!(jev.calls, 1);
    assert_eq!(
        store.tasks_for_day("2026-09-02").unwrap()[0].state,
        State::Done
    );
}

#[test]
fn jev_outage_still_applies_a_known_edge() {
    let home = Home::new();
    let store = home.store();
    let t = store
        .add_task("経費精算", None, None, "manual", None, Some("2026-09-01"))
        .unwrap();
    store.carry_over(&t.id, "no_time", "2026-09-02", None).unwrap();
    store
        .add_task("未経験", None, None, "manual", None, Some("2026-09-02"))
        .unwrap();
    let mut jev = Scripted {
        choice: "done".into(),
        confidence: 0.99,
        calls: 0,
        down: true,
    };
    let proposed = jev::propose_unknown(&store, "2026-09-02", &mut jev, 0.5).unwrap();
    assert!(store
        .tasks_for_day("2026-09-02")
        .unwrap()
        .iter()
        .any(|t| t.title == "経費精算" && t.state == State::Carried));
    assert_eq!(proposed.len(), 1);
    assert_eq!(proposed[0].choice, "ask");
}

#[test]
fn close_yes_stays_pending_when_a_node_is_unknown() {
    let home = Home::new();
    let store = home.store();
    store
        .add_task("初めて", None, None, "manual", None, Some("2026-09-03"))
        .unwrap();
    let outcome = rituals::close_ritual(&store, &Ui::new(true), "2026-09-03").unwrap();
    assert!(matches!(outcome, Outcome::Pending(1)));
}

#[test]
fn tool_descriptions_do_not_mention_resolve_with() {
    for tool in tools::list() {
        let name = tool["name"].as_str().unwrap();
        let description = tool["description"].as_str().unwrap();
        assert!(
            !description.contains("resolve_with"),
            "{name} still says resolve_with"
        );
    }
}

#[test]
fn close_question_labels_come_from_the_engine() {
    let task = dayloop::model::Task {
        id: "01TEST".into(),
        title: "件".into(),
        source: "manual".into(),
        source_ref: None,
        due: None,
        estimate_min: None,
        plan_date: Some("2026-09-03".into()),
        state: State::Planned,
        state_reason: None,
        carried_count: 0,
        evidence: None,
        created_at: "2026-09-03T00:00:00+09:00".into(),
        closed_at: None,
        carried_from: None,
        proposed_state: None,
        proposed_reason_code: None,
        proposal_confidence: None,
        state_note: None,
        decided_by: None,
    };
    let q = Question::close_task(&task);
    let labels: Vec<&str> = q.options.iter().map(|o| o.label.as_str()).collect();
    assert_eq!(labels, ["完了", "未完了", "持ち越し", "取り下げ"]);
}

#[test]
fn due_and_fit_are_computed_in_rust() {
    assert_eq!(
        facts::due_relation("2026-09-22", Some("2026-09-21")),
        DueRelation::Overdue
    );
    assert_eq!(facts::due_relation("2026-09-22", None), DueRelation::None);
    let meeting = facts::span("10:00", "11:00").unwrap();
    assert_eq!(facts::fit_estimate(Some(30), &[meeting]), Fit::Fits);
    assert_eq!(facts::fit_estimate(Some(30), &[]), Fit::Unknown);
    let packed = [
        facts::span("09:30", "12:00").unwrap(),
        facts::span("12:30", "18:00").unwrap(),
    ];
    assert_eq!(facts::fit_estimate(Some(120), &packed), Fit::Over);
}
