//! Today's order. Rust ranks the work. Jev only hints on one tie.

use anyhow::Result;
use chrono::NaiveTime;

use crate::facts::{self, DueRelation, Fit};
use crate::model::{State, Task};
use crate::store::Store;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Rank {
    due_class: u8,
    carry_key: i64,
    fit_class: u8,
    estimate_key: i64,
}

pub fn spans_hhmm(pairs: &[(&str, &str)]) -> Vec<(NaiveTime, NaiveTime)> {
    pairs.iter().filter_map(|(a, b)| facts::span(a, b)).collect()
}

pub fn spans_from_events(events: &[crate::model::Event]) -> Vec<(NaiveTime, NaiveTime)> {
    events.iter().filter_map(|e| facts::span(&e.start, &e.end)).collect()
}

pub fn rank(today: &str, task: &Task, spans: &[(NaiveTime, NaiveTime)]) -> Rank {
    let due = facts::due_relation(today, task.due.as_deref());
    let due_class = match due {
        DueRelation::Overdue => 0,
        DueRelation::Today => 1,
        DueRelation::None => 2,
        DueRelation::Future => 3,
    };
    let fit = facts::fit_estimate(task.estimate_min, spans);
    let fit_class = match fit {
        Fit::Fits => 0,
        Fit::Unknown => 1,
        Fit::Over => 2,
    };
    Rank {
        due_class,
        carry_key: -task.carried_count,
        fit_class,
        estimate_key: task.estimate_min.unwrap_or(i64::MAX),
    }
}

pub fn sort_open(today: &str, mut tasks: Vec<Task>, spans: &[(NaiveTime, NaiveTime)]) -> Vec<Task> {
    tasks.sort_by(|a, b| {
        rank(today, a, spans)
            .cmp(&rank(today, b, spans))
            .then_with(|| a.created_at.cmp(&b.created_at))
            .then_with(|| a.id.cmp(&b.id))
    });
    tasks
}

pub fn sort_day(today: &str, mut tasks: Vec<Task>, spans: &[(NaiveTime, NaiveTime)]) -> Vec<Task> {
    tasks.sort_by(|a, b| {
        open_key(a.state)
            .cmp(&open_key(b.state))
            .then_with(|| rank(today, a, spans).cmp(&rank(today, b, spans)))
            .then_with(|| a.created_at.cmp(&b.created_at))
            .then_with(|| a.id.cmp(&b.id))
    });
    tasks
}

fn open_key(state: State) -> u8 {
    if state.is_open() { 0 } else { 1 }
}

pub fn first_open_tie<'a>(
    today: &str,
    tasks: &'a [Task],
    spans: &[(NaiveTime, NaiveTime)],
) -> Option<(&'a Task, &'a Task)> {
    let open: Vec<&Task> = tasks.iter().filter(|t| t.state.is_open()).collect();
    open.windows(2).find_map(|w| {
        if rank(today, w[0], spans) == rank(today, w[1], spans) {
            Some((w[0], w[1]))
        } else {
            None
        }
    })
}

pub fn day_tasks(store: &Store, date: &str) -> Result<Vec<Task>> {
    let tasks = store.tasks_for_day(date)?;
    let spans = spans_from_events(&store.events_for_day(date)?);
    let mut tasks = sort_day(date, tasks, &spans);
    apply_saved_first_tie(store, date, &spans, &mut tasks)?;
    Ok(tasks)
}

fn apply_saved_first_tie(
    store: &Store,
    today: &str,
    spans: &[(NaiveTime, NaiveTime)],
    tasks: &mut [Task],
) -> Result<()> {
    let pair = first_open_tie(today, tasks, spans).map(|(left, right)| {
        (
            left.id.clone(),
            right.id.clone(),
            left.title.clone(),
            right.title.clone(),
        )
    });
    let Some((left_id, right_id, left_title, right_title)) = pair else {
        return Ok(());
    };
    let Some(first) = crate::graph::saved_first(store, &left_title, &right_title)? else {
        return Ok(());
    };
    if first != right_title {
        return Ok(());
    }
    if let Some(i) = tasks.iter().position(|t| t.id == left_id) {
        if tasks.get(i + 1).is_some_and(|t| t.id == right_id) {
            tasks.swap(i, i + 1);
        }
    }
    Ok(())
}

pub fn next_open(store: &Store, date: &str) -> Result<Option<Task>> {
    Ok(day_tasks(store, date)?.into_iter().find(|t| t.state.is_open()))
}

pub fn tie_hint(decider: &mut dyn crate::jev::Decider, a: &str, b: &str) -> Option<String> {
    let choices = vec![a.to_string(), b.to_string()];
    let state = format!("order a={a} b={b}");
    match decider.decide(&state, &choices) {
        crate::jev::JevOutcome::Answer { choice, .. } if choice == a || choice == b => Some(choice),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{State, Task};

    fn task(
        id: &str,
        title: &str,
        due: Option<&str>,
        estimate: Option<i64>,
        carried: i64,
        created: &str,
    ) -> Task {
        Task {
            id: id.into(),
            title: title.into(),
            source: "manual".into(),
            source_ref: None,
            due: due.map(|s| s.to_string()),
            estimate_min: estimate,
            plan_date: Some("2026-09-22".into()),
            state: State::Planned,
            state_reason: None,
            carried_count: carried,
            evidence: None,
            created_at: created.into(),
            closed_at: None,
            carried_from: None,
            proposed_state: None,
            proposed_reason_code: None,
            proposal_confidence: None,
            state_note: None,
            decided_by: None,
        }
    }

    #[test]
    fn overdue_before_today_before_future() {
        let spans = spans_hhmm(&[("10:00", "11:00")]);
        let tasks = vec![
            task("c", "未来", Some("2026-09-23"), Some(30), 0, "2026-09-22T01:00:00Z"),
            task("a", "超過", Some("2026-09-21"), Some(30), 0, "2026-09-22T03:00:00Z"),
            task("b", "今日", Some("2026-09-22"), Some(30), 0, "2026-09-22T02:00:00Z"),
        ];
        let ids: Vec<_> = sort_open("2026-09-22", tasks, &spans)
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn higher_carry_wins_inside_the_same_due_class() {
        let tasks = vec![
            task("new", "新しい", Some("2026-09-21"), Some(30), 0, "2026-09-22T01:00:00Z"),
            task("old", "古い", Some("2026-09-21"), Some(30), 3, "2026-09-22T02:00:00Z"),
        ];
        let ids: Vec<_> = sort_open("2026-09-22", tasks, &[])
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(ids, vec!["old", "new"]);
    }

    #[test]
    fn fits_before_unknown_before_over() {
        let spans = spans_hhmm(&[("09:00", "17:30")]);
        let tasks = vec![
            task("over", "入らない", Some("2026-09-22"), Some(60), 0, "2026-09-22T01:00:00Z"),
            task("fit", "入る", Some("2026-09-22"), Some(30), 0, "2026-09-22T02:00:00Z"),
            task("unk", "見積なし", Some("2026-09-22"), None, 0, "2026-09-22T00:00:00Z"),
        ];
        let ids: Vec<_> = sort_open("2026-09-22", tasks, &spans)
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(ids, vec!["fit", "unk", "over"]);
    }

    #[test]
    fn equal_rank_keeps_created_at_then_id() {
        let tasks = vec![
            task("b", "同着", Some("2026-09-22"), Some(30), 0, "2026-09-22T02:00:00Z"),
            task("a", "同着", Some("2026-09-22"), Some(30), 0, "2026-09-22T02:00:00Z"),
        ];
        let ids: Vec<_> = sort_open("2026-09-22", tasks, &[])
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(ids, vec!["a", "b"]);
    }

    struct Script {
        choice: String,
        calls: usize,
    }

    impl crate::jev::Decider for Script {
        fn decide(&mut self, _state: &str, choices: &[String]) -> crate::jev::JevOutcome {
            self.calls += 1;
            assert_eq!(choices, ["甲".to_string(), "乙".to_string()]);
            crate::jev::JevOutcome::Answer {
                choice: self.choice.clone(),
                confidence: 1.0,
            }
        }
    }

    #[test]
    fn tie_hint_returns_only_one_of_the_two_titles() {
        let mut decider = Script {
            choice: "乙".into(),
            calls: 0,
        };
        let hint = tie_hint(&mut decider, "甲", "乙");
        assert_eq!(hint, Some("乙".into()));
        assert_eq!(decider.calls, 1);
    }
}
