use anyhow::Result;
use chrono::{DateTime, Local, TimeZone};
use dayloop::config::{Config, Notify, Schedule};
use dayloop::notify::Sent;
use dayloop::rituals::Outcome;
use dayloop::serve::{run_once_at, Phase};
use std::path::PathBuf;

struct Home(PathBuf);

impl Home {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("dayloop-serve-{}", ulid::Ulid::new()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn state_path(&self) -> PathBuf {
        self.0.join("serve-state.json")
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn cfg() -> Config {
    Config {
        schedule: Schedule {
            plan: "08:00".into(),
            check: "12:00".into(),
            close: "18:00".into(),
            retro: "Fri 18:00".into(),
            workdays: ["Mon", "Tue", "Wed", "Thu", "Fri"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        },
        notify: Notify {
            method: "file".into(),
        },
        ..Config::default()
    }
}

fn at(y: i32, m: u32, d: u32, h: u32, minute: u32) -> DateTime<Local> {
    Local
        .with_ymd_and_hms(y, m, d, h, minute, 0)
        .single()
        .unwrap()
}

#[test]
fn pending_is_delivered_once_per_slot_and_friday_retro_is_grouped_with_evening() {
    let home = Home::new();
    let mut messages = Vec::new();
    let evaluate = |_phase: Phase, _date: &str| -> Result<Outcome> { Ok(Outcome::Pending(1)) };
    let notify = |message: &str| -> Result<Sent> {
        messages.push(message.to_owned());
        Ok(Sent::FileOk)
    };
    run_once_at(
        &cfg(),
        &home.state_path(),
        at(2026, 9, 4, 8, 0),
        evaluate,
        notify,
    )
    .unwrap();
    assert_eq!(messages.len(), 1);

    run_once_at(
        &cfg(),
        &home.state_path(),
        at(2026, 9, 4, 8, 30),
        |_phase, _date| Ok(Outcome::Pending(1)),
        |_message| Ok(Sent::FileOk),
    )
    .unwrap();
    assert_eq!(messages.len(), 1);

    let noon = run_once_at(
        &cfg(),
        &home.state_path(),
        at(2026, 9, 4, 12, 0),
        |_phase, _date| Ok(Outcome::Pending(1)),
        |message| {
            messages.push(message.to_owned());
            Ok(Sent::FileOk)
        },
    )
    .unwrap();
    assert!(noon.notification_sent);
    assert!(messages[1].contains("計画") && messages[1].contains("途中確認"));

    run_once_at(
        &cfg(),
        &home.state_path(),
        at(2026, 9, 4, 18, 0),
        |_phase, _date| Ok(Outcome::Pending(1)),
        |message| {
            messages.push(message.to_owned());
            Ok(Sent::FileOk)
        },
    )
    .unwrap();
    assert_eq!(messages.len(), 3);
    assert!(messages[2].contains("クローズ") && messages[2].contains("振り返り"));
}

#[test]
fn catch_up_groups_elapsed_phases_once_and_delivery_failure_retries_at_later_slot() {
    let home = Home::new();
    let mut messages = Vec::new();
    let catch_up = run_once_at(
        &cfg(),
        &home.state_path(),
        at(2026, 9, 3, 18, 5),
        |_phase, _date| Ok(Outcome::Pending(1)),
        |message| {
            messages.push(message.to_owned());
            Ok(Sent::FileOk)
        },
    )
    .unwrap();
    assert!(catch_up.notification_sent);
    assert_eq!(
        catch_up.evaluated,
        vec![Phase::Plan, Phase::Check, Phase::Close]
    );
    assert_eq!(messages.len(), 1);
    assert!(
        messages[0].contains("計画")
            && messages[0].contains("途中確認")
            && messages[0].contains("クローズ")
    );

    let retry_home = Home::new();
    let failure = run_once_at(
        &cfg(),
        &retry_home.state_path(),
        at(2026, 9, 3, 8, 0),
        |_phase, _date| Ok(Outcome::Pending(1)),
        |_message| anyhow::bail!("synthetic write failure"),
    )
    .unwrap();
    assert!(failure.delivery_error.is_some());
    assert!(!failure.notification_sent);

    let same_slot_restart = run_once_at(
        &cfg(),
        &retry_home.state_path(),
        at(2026, 9, 3, 8, 30),
        |_phase, _date| Ok(Outcome::Pending(1)),
        |_message| Ok(Sent::FileOk),
    )
    .unwrap();
    assert!(!same_slot_restart.notification_sent);

    let mut retried = Vec::new();
    let next_slot = run_once_at(
        &cfg(),
        &retry_home.state_path(),
        at(2026, 9, 3, 12, 0),
        |phase, _date| {
            retried.push(phase);
            anyhow::bail!("synthetic evaluation error")
        },
        |message| {
            messages.push(message.to_owned());
            Ok(Sent::FileOk)
        },
    )
    .unwrap();
    assert!(!next_slot.notification_sent);
    assert_eq!(retried, vec![Phase::Plan, Phase::Check]);
    assert_eq!(messages.len(), 1);

    let evening_retry = run_once_at(
        &cfg(),
        &retry_home.state_path(),
        at(2026, 9, 3, 18, 0),
        |_phase, _date| Ok(Outcome::Pending(1)),
        |message| {
            messages.push(message.to_owned());
            Ok(Sent::FileOk)
        },
    )
    .unwrap();
    assert!(evening_retry.notification_sent);
    assert_eq!(messages.len(), 2);
}

#[test]
fn later_evaluation_clears_a_stale_pending_reminder_when_answered() {
    let home = Home::new();
    let mut messages = Vec::new();
    run_once_at(
        &cfg(),
        &home.state_path(),
        at(2026, 9, 3, 8, 0),
        |_phase, _date| Ok(Outcome::Pending(1)),
        |message| {
            messages.push(message.to_owned());
            Ok(Sent::FileOk)
        },
    )
    .unwrap();
    run_once_at(
        &cfg(),
        &home.state_path(),
        at(2026, 9, 3, 12, 0),
        |_phase, _date| Ok(Outcome::Done),
        |message| {
            messages.push(message.to_owned());
            Ok(Sent::FileOk)
        },
    )
    .unwrap();
    assert_eq!(
        messages.len(),
        1,
        "answered plan must not be reminded at noon"
    );
}

#[test]
fn friday_mismatched_retro_time_defers_close_until_one_evening_group() {
    let home = Home::new();
    let mut late_cfg = cfg();
    late_cfg.schedule.retro = "Fri 18:30".into();
    let mut messages = Vec::new();
    run_once_at(
        &late_cfg,
        &home.state_path(),
        at(2026, 9, 4, 12, 0),
        |_phase, _date| Ok(Outcome::Done),
        |_message| Ok(Sent::FileOk),
    )
    .unwrap();
    let at_close = run_once_at(
        &late_cfg,
        &home.state_path(),
        at(2026, 9, 4, 18, 0),
        |_phase, _date| Ok(Outcome::Pending(1)),
        |message| {
            messages.push(message.to_owned());
            Ok(Sent::FileOk)
        },
    )
    .unwrap();
    assert!(!at_close.notification_sent);
    let evening = run_once_at(
        &late_cfg,
        &home.state_path(),
        at(2026, 9, 4, 18, 30),
        |_phase, _date| Ok(Outcome::Pending(1)),
        |message| {
            messages.push(message.to_owned());
            Ok(Sent::FileOk)
        },
    )
    .unwrap();
    assert!(evening.notification_sent);
    assert_eq!(messages.len(), 1);
    assert!(messages[0].contains("クローズ") && messages[0].contains("振り返り"));
}

#[test]
fn before_first_slot_does_not_create_state() {
    let home = Home::new();
    let state_path = home.state_path();
    let tick = run_once_at(
        &cfg(),
        &state_path,
        at(2026, 9, 3, 7, 59),
        |_phase, _date| Ok(Outcome::Pending(1)),
        |_message| Ok(Sent::FileOk),
    )
    .unwrap();
    assert!(tick.evaluated.is_empty());
    assert!(!state_path.exists());
}

#[test]
fn friday_pending_is_reminded_on_monday_with_its_original_date() {
    let home = Home::new();
    let mut messages = Vec::new();
    run_once_at(
        &cfg(),
        &home.state_path(),
        at(2026, 9, 4, 18, 0),
        |phase, _date| {
            if phase == Phase::Close {
                Ok(Outcome::Pending(1))
            } else {
                Ok(Outcome::Done)
            }
        },
        |_message| Ok(Sent::FileOk),
    )
    .unwrap();
    run_once_at(
        &cfg(),
        &home.state_path(),
        at(2026, 9, 7, 8, 0),
        |phase, date| {
            if phase == Phase::Close && date == "2026-09-04" {
                Ok(Outcome::Pending(1))
            } else {
                Ok(Outcome::Done)
            }
        },
        |message| {
            messages.push(message.to_owned());
            Ok(Sent::FileOk)
        },
    )
    .unwrap();
    assert_eq!(messages.len(), 1);
    assert!(messages[0].contains("2026-09-04 のクローズ"));
}

#[test]
fn next_friday_keeps_an_old_retro_alongside_the_new_week() {
    let home = Home::new();
    run_once_at(
        &cfg(),
        &home.state_path(),
        at(2026, 9, 4, 18, 0),
        |phase, _| {
            if phase == Phase::Retro {
                Ok(Outcome::Pending(1))
            } else {
                Ok(Outcome::Done)
            }
        },
        |_message| Ok(Sent::FileOk),
    )
    .unwrap();
    let mut messages = Vec::new();
    run_once_at(
        &cfg(),
        &home.state_path(),
        at(2026, 9, 11, 18, 0),
        |phase, _| {
            if phase == Phase::Retro {
                Ok(Outcome::Pending(1))
            } else {
                Ok(Outcome::Done)
            }
        },
        |message| {
            messages.push(message.to_owned());
            Ok(Sent::FileOk)
        },
    )
    .unwrap();
    assert_eq!(messages.len(), 1);
    assert!(messages[0].contains("2026-09-04 の振り返り"));
    assert!(messages[0].contains("2026-09-11 の振り返り"));
}

#[test]
fn old_answered_phase_is_removed_from_history() {
    let home = Home::new();
    run_once_at(
        &cfg(),
        &home.state_path(),
        at(2026, 9, 4, 18, 0),
        |phase, _| {
            if phase == Phase::Close {
                Ok(Outcome::Pending(1))
            } else {
                Ok(Outcome::Done)
            }
        },
        |_message| Ok(Sent::FileOk),
    )
    .unwrap();
    let monday = run_once_at(
        &cfg(),
        &home.state_path(),
        at(2026, 9, 7, 8, 0),
        |_phase, _date| Ok(Outcome::Done),
        |_message| Ok(Sent::FileOk),
    )
    .unwrap();
    assert!(!monday.notification_sent);
    let saved = dayloop::serve::ServeState::load_at(&home.state_path()).unwrap();
    assert!(!saved.pending_history.contains_key("2026-09-04/close"));
}

#[test]
fn daily_rollover_keeps_old_and_current_pending_separately() {
    let home = Home::new();
    run_once_at(
        &cfg(),
        &home.state_path(),
        at(2026, 9, 4, 8, 0),
        |_phase, _| Ok(Outcome::Pending(1)),
        |_message| Ok(Sent::FileOk),
    )
    .unwrap();
    run_once_at(
        &cfg(),
        &home.state_path(),
        at(2026, 9, 7, 8, 0),
        |_phase, _| Ok(Outcome::Pending(1)),
        |_message| Ok(Sent::FileOk),
    )
    .unwrap();
    let saved = dayloop::serve::ServeState::load_at(&home.state_path()).unwrap();
    assert!(saved.pending_history.contains_key("2026-09-04/plan"));
    assert_eq!(
        saved
            .phase_state(Phase::Plan)
            .and_then(|item| item.pending_on.as_deref()),
        Some("2026-09-07")
    );
}

#[test]
fn legacy_old_phase_state_is_migrated_without_losing_the_pending_work() {
    let home = Home::new();
    std::fs::write(
        home.state_path(),
        r#"{"phases":{"plan":{"evaluated_on":"2026-09-04","pending_on":"2026-09-04","pending_count":1}}}"#,
    )
    .unwrap();
    let mut messages = Vec::new();
    run_once_at(
        &cfg(),
        &home.state_path(),
        at(2026, 9, 7, 8, 0),
        |phase, date| {
            if phase == Phase::Plan && date == "2026-09-04" {
                Ok(Outcome::Pending(1))
            } else {
                Ok(Outcome::Done)
            }
        },
        |message| {
            messages.push(message.to_owned());
            Ok(Sent::FileOk)
        },
    )
    .unwrap();
    assert_eq!(messages.len(), 1);
    assert!(messages[0].contains("2026-09-04 の計画"));
    let saved = dayloop::serve::ServeState::load_at(&home.state_path()).unwrap();
    assert!(saved.pending_history.contains_key("2026-09-04/plan"));
}

#[test]
fn weekend_does_not_deliver_daily_history_without_a_retro_occurrence() {
    let home = Home::new();
    run_once_at(
        &cfg(),
        &home.state_path(),
        at(2026, 9, 4, 8, 0),
        |_phase, _| Ok(Outcome::Pending(1)),
        |_message| Ok(Sent::FileOk),
    )
    .unwrap();
    let saturday = run_once_at(
        &cfg(),
        &home.state_path(),
        at(2026, 9, 5, 18, 0),
        |_phase, _| Ok(Outcome::Pending(1)),
        |_message| Ok(Sent::FileOk),
    )
    .unwrap();
    assert!(saturday.evaluated.is_empty());
    assert!(!saturday.notification_sent);
}
