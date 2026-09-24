//! Work-step graph: known edges skip Jev, a confident answer becomes the next edge.

use std::sync::{Mutex, MutexGuard};

use dayloop::browse::judge_visible;
use dayloop::graph::{self, seed_work_steps};
use dayloop::jev::{self, Decider, JevOutcome};
use dayloop::screen::{self, ReadFail, ScreenReader};
use dayloop::store::Store;

static LOCK: Mutex<()> = Mutex::new(());

struct Home {
    dir: std::path::PathBuf,
    prev: Option<String>,
    _lock: MutexGuard<'static, ()>,
}

impl Home {
    fn new() -> Self {
        let lock = LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let dir = std::env::temp_dir().join(format!("dayloop-steps-{}", ulid::Ulid::new()));
        std::fs::create_dir_all(&dir).unwrap();
        let prev = std::env::var("DAYLOOP_HOME").ok();
        std::env::set_var("DAYLOOP_HOME", &dir);
        Self { dir, prev, _lock: lock }
    }
    fn store(&self) -> Store {
        Store::open().unwrap()
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        match &self.prev {
            Some(value) => std::env::set_var("DAYLOOP_HOME", value),
            None => std::env::remove_var("DAYLOOP_HOME"),
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

struct Boom;
impl Decider for Boom {
    fn decide(&mut self, _: &str, _: &[String]) -> JevOutcome {
        panic!("jev should not be called");
    }
}

struct Once {
    calls: usize,
}
impl Decider for Once {
    fn decide(&mut self, _: &str, choices: &[String]) -> JevOutcome {
        self.calls += 1;
        JevOutcome::Answer { choice: choices[0].clone(), confidence: 0.9 }
    }
}

struct Five {
    calls: usize,
}
impl Decider for Five {
    fn decide(&mut self, _: &str, _: &[String]) -> JevOutcome {
        self.calls += 1;
        let choice = match self.calls {
            1 => "request",
            2 => "needs_check",
            3 => "today",
            4 => "new",
            _ => "ask_person",
        };
        JevOutcome::Answer { choice: choice.into(), confidence: 0.8 }
    }
}

struct Low;
impl Decider for Low {
    fn decide(&mut self, _: &str, _: &[String]) -> JevOutcome {
        JevOutcome::Answer { choice: "keep".into(), confidence: 0.1 }
    }
}

struct Fixed {
    fg: String,
    read: String,
}
impl ScreenReader for Fixed {
    fn foreground_json(&mut self) -> Result<String, ReadFail> {
        Ok(self.fg.clone())
    }
    fn read_json(&mut self, _: &str) -> Result<String, ReadFail> {
        Ok(self.read.clone())
    }
}

fn window(process: &str, title: &str, text: &str) -> Fixed {
    Fixed {
        fg: serde_json::json!({
            "success": true,
            "window": {"handle": "42", "title": title, "processName": process, "isElevated": false}
        })
        .to_string(),
        read: serde_json::json!({"success": true, "text": text, "hint": ""}).to_string(),
    }
}

#[test]
fn five_known_edges_are_seeded_once() {
    let home = Home::new();
    let store = home.store();
    seed_work_steps(&store).unwrap();
    let listed = graph::list_steps(&store).unwrap();
    for key in [
        "capture:title_only",
        "boards:toolbar",
        "github:credential",
        "devops:api_denied",
        "capture:body",
        "capture:other_app",
        "browse:language",
    ] {
        assert!(listed.contains(key), "{listed}");
    }
    assert_eq!(graph::follow_step(&store, "capture:body").unwrap().as_deref(), Some("hold_send"));
    assert_eq!(graph::follow_step(&store, "github:credential").unwrap().as_deref(), Some("gh"));
    assert_eq!(graph::follow_step(&store, "devops:api_denied").unwrap().as_deref(), Some("skip"));
    assert_eq!(graph::follow_step(&store, "boards:toolbar").unwrap().as_deref(), Some("strip"));
    let decision = jev::decide_step(&store, "capture:title_only", "題名だけ", &["fail", "keep"], &mut Boom, 0.5).unwrap();
    assert!(decision.from_graph);
    assert_eq!(decision.choice.as_deref(), Some("fail"));
}

#[test]
fn a_new_step_asks_once_and_a_weak_answer_does_not_stick() {
    let home = Home::new();
    let store = home.store();
    let mut once = Once { calls: 0 };
    let first = jev::decide_step(&store, "browse:gmail:見積の確認", "見積", &["keep", "skip"], &mut once, 0.5).unwrap();
    assert!(!first.from_graph);
    assert_eq!(first.choice.as_deref(), Some("keep"));
    assert_eq!(once.calls, 1);
    let second = jev::decide_step(&store, "browse:gmail:見積の確認", "見積", &["keep", "skip"], &mut Boom, 0.5).unwrap();
    assert!(second.from_graph);
    assert_eq!(second.choice.as_deref(), Some("keep"));

    let weak = jev::decide_step(&store, "browse:gmail:弱い件", "弱い", &["keep", "skip"], &mut Low, 0.5).unwrap();
    assert!(weak.choice.is_none());
    assert!(graph::find_step(&store, "browse:gmail:弱い件").unwrap().is_none());
}

#[test]
fn the_same_visible_subject_and_the_same_screen_text_skip_jev() {
    let home = Home::new();
    let store = home.store();
    let mut once = Once { calls: 0 };
    let (keep, via) = judge_visible(&store, "gmail", "見積の確認", "見積の確認をお願いします", &[], &mut once, 0.5).unwrap();
    assert!(keep);
    assert_eq!(via, "Jev");
    let (again, via) = judge_visible(&store, "gmail", "見積の確認", "見積の確認をお願いします", &[], &mut Boom, 0.5).unwrap();
    assert!(again);
    assert_eq!(via, "グラフ");

    let body = "9月25日15時までに、LINE連携の仕様書をレビューしてください";
    let mut first = window("olk.exe", "受信トレイ - Outlook", body);
    let mut jev = Five { calls: 0 };
    let report = screen::capture(&store, &mut first, true, Some(&mut jev)).unwrap();
    assert!(report.contains("対応依頼"));
    assert_eq!(jev.calls, 5);
    let mut second = window("olk.exe", "受信トレイ - Outlook", body);
    let report = screen::capture(&store, &mut second, true, Some(&mut Boom)).unwrap();
    assert!(report.contains("グラフ: 同じ文章なので再判断しません"));
}

#[test]
fn title_only_follows_fail_until_the_edge_says_keep() {
    let home = Home::new();
    let store = home.store();
    let sentence = "9月25日までに仕様書をレビューしてください";
    let mut reader = window("olk.exe", sentence, sentence);
    let failed = screen::capture(&store, &mut reader, false, None).unwrap();
    assert!(failed.contains("取得失敗"));
    graph::remember_step(&store, "capture:title_only", "keep", Some("person")).unwrap();
    let mut reader = window("olk.exe", sentence, sentence);
    let kept = screen::capture(&store, &mut reader, false, None).unwrap();
    assert!(!kept.contains("取得失敗"));
    assert!(kept.contains(sentence));
}
