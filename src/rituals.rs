//! The daily loop: plan (morning) → check (midday) → close (evening) → retro (weekly).
//! Every decision is made by the person; the code only refuses to move on while
//! something is still undecided.

use std::io::{self, BufRead, IsTerminal, Write};

use anyhow::Result;

use crate::engine::Question;
use crate::graph;
use crate::model::{State, Task};
use crate::store::{CarryBlocked, Store, CANDIDATE_STALE_DAYS, MAX_CARRY};
use crate::util::{days_since, hhmm, next_workday, parse_date, short, week_range};

pub struct Ui {
    interactive: bool,
}

impl Ui {
    pub fn new(yes: bool) -> Ui {
        // DAYLOOP_INTERACTIVE=1 forces prompts even when stdin is piped (tests, wrappers).
        let forced = std::env::var("DAYLOOP_INTERACTIVE").map(|v| v == "1").unwrap_or(false);
        Ui {
            interactive: !yes && (forced || io::stdin().is_terminal()),
        }
    }

    /// One question, up to 4 options, first is the default. None = not answered.
    pub fn choose(&self, q: &str, opts: &[&str]) -> Option<usize> {
        println!();
        println!("? {q}");
        for (i, o) in opts.iter().enumerate() {
            println!("  {}) {}{}", i + 1, o, if i == 0 { "  (Enter で既定)" } else { "" });
        }
        if !self.interactive {
            println!("  -> 対話できないため未回答のまま残します");
            return None;
        }
        loop {
            print!("  > ");
            io::stdout().flush().ok();
            let mut s = String::new();
            match io::stdin().lock().read_line(&mut s) {
                Ok(0) | Err(_) => {
                    // EOF: nobody answered. Never fall back to the default here.
                    println!("(入力終了。未回答のまま残します)");
                    return None;
                }
                Ok(_) => {}
            }
            let s = s.trim();
            if s.is_empty() {
                return Some(0);
            }
            if let Ok(n) = s.parse::<usize>() {
                if (1..=opts.len()).contains(&n) {
                    return Some(n - 1);
                }
            }
            println!("  1〜{} で答えてください", opts.len());
        }
    }

    /// Free text; empty allowed.
    pub fn line(&self, q: &str) -> Option<String> {
        println!();
        println!("? {q}");
        if !self.interactive {
            println!("  -> 対話できないため未回答のまま残します");
            return None;
        }
        print!("  > ");
        io::stdout().flush().ok();
        let mut s = String::new();
        match io::stdin().lock().read_line(&mut s) {
            Ok(0) | Err(_) => {
                println!("(入力終了。未回答のまま残します)");
                None
            }
            Ok(_) => Some(s.trim().to_string()),
        }
    }

    /// Free text; must be non-empty.
    pub fn text(&self, q: &str) -> Option<String> {
        loop {
            let s = self.line(q)?;
            if !s.is_empty() {
                return Some(s);
            }
            println!("  空にはできません");
        }
    }
}

pub enum Outcome {
    Done,
    /// Number of items still waiting for the person.
    Pending(usize),
}

fn meta(t: &Task) -> String {
    let mut m = Vec::new();
    if let Some(d) = &t.due {
        m.push(format!("期限 {d}"));
    }
    if let Some(e) = t.estimate_min {
        m.push(format!("{e}分"));
    }
    if t.carried_count > 0 {
        m.push(format!("持ち越し{}回", t.carried_count));
    }
    if t.source != "manual" {
        m.push(t.source.clone());
    }
    if m.is_empty() {
        String::new()
    } else {
        format!("  ({})", m.join("・"))
    }
}

pub fn print_day(store: &Store, date: &str) -> Result<()> {
    let day = store.get_day(date)?;
    let tasks = crate::order::day_tasks(store, date)?;
    let open = tasks.iter().filter(|t| t.state.is_open()).count();
    println!("== {date}");
    match &day {
        Some(d) if d.closed_at.is_some() => {
            println!("   クローズ済み {}", hhmm(d.closed_at.as_deref().unwrap_or("")))
        }
        Some(d) if d.plan_confirmed_at.is_some() => println!(
            "   計画確定 {} / 未確定 {open} 件",
            hhmm(d.plan_confirmed_at.as_deref().unwrap_or(""))
        ),
        _ => println!("   計画未確定 / 未確定 {open} 件"),
    }
    let events = store.events_for_day(date)?;
    if !events.is_empty() {
        println!("   [今日の予定]");
        for e in &events {
            let loc = e.location.as_deref().map(|l| format!("  {l}")).unwrap_or_default();
            println!(
                "     {}-{}  {}{}",
                hhmm(&e.start),
                hhmm(&e.end),
                e.subject,
                loc
            );
        }
    }
    if tasks.is_empty() {
        println!("   タスクなし");
    }
    let order = [
        State::InProgress,
        State::Planned,
        State::Done,
        State::NotDone,
        State::Carried,
        State::Dropped,
    ];
    for st in order {
        let group: Vec<&Task> = tasks.iter().filter(|t| t.state == st).collect();
        if group.is_empty() {
            continue;
        }
        println!("   [{}]", st.label_ja());
        for t in group {
            let overdue = match (&t.due, st.is_open()) {
                (Some(d), true) if d.as_str() < date => "  !! 期限超過",
                _ => "",
            };
            let reason = t
                .state_reason
                .as_ref()
                .map(|r| format!("  理由: {r}"))
                .unwrap_or_default();
            println!("     {}  {}{}{}{}", short(&t.id), t.title, meta(t), overdue, reason);
        }
    }
    let cands = store.open_candidates()?;
    if !cands.is_empty() {
        let stale = cands
            .iter()
            .filter(|c| days_since(&c.created_at) >= CANDIDATE_STALE_DAYS)
            .count();
        println!(
            "   候補 {} 件が採用/却下待ち{}",
            cands.len(),
            if stale > 0 { format!("（うち {stale} 件が {CANDIDATE_STALE_DAYS} 日以上放置）") } else { String::new() }
        );
    }
    let unclosed = store.unclosed_days_before(date)?;
    if !unclosed.is_empty() {
        println!("   !! 未クローズの日があります: {}", unclosed.join(", "));
    }
    Ok(())
}

/// Resolve one task that must leave the open state. Returns false if left undecided.
fn carry_flow(store: &Store, ui: &Ui, t: &Task, date: &str) -> Result<bool> {
    let to = next_workday(date)?;
    if t.carried_count >= MAX_CARRY {
        println!("  「{}」は既に {} 回持ち越されています。", t.title, t.carried_count);
        return resolve_blocked(store, ui, t, &to);
    }
    let Some(reason) = ui.text("持ち越す理由") else { return Ok(false) };
    match store.carry_over(&t.id, &reason, &to, None) {
        Ok(n) => {
            println!("  -> {to} に持ち越し（{}回目）", n.carried_count);
            Ok(true)
        }
        Err(e) if e.downcast_ref::<CarryBlocked>().is_some() => resolve_blocked(store, ui, t, &to),
        Err(e) => Err(e),
    }
}

/// Invariant 3: split / drop / reschedule.
fn resolve_blocked(store: &Store, ui: &Ui, t: &Task, to: &str) -> Result<bool> {
    let q = Question::carry_blocked(t);
    let labels = option_labels(&q);
    match ui.choose(&q.question, &labels) {
        Some(0) => {
            let mut titles = Vec::new();
            loop {
                let Some(s) = ui.line(&format!("分割後のタスク {}（空行で終了）", titles.len() + 1)) else {
                    return Ok(false);
                };
                if s.is_empty() {
                    if titles.len() >= 2 {
                        break;
                    }
                    println!("  2 つ以上に分けてください");
                    continue;
                }
                titles.push(s);
            }
            let news = store.split(&t.id, &titles, "持ち越し上限", to)?;
            println!("  -> {} 件に分割し {to} の予定にしました", news.len());
            Ok(true)
        }
        Some(1) => {
            let Some(r) = ui.text("取り下げる理由") else { return Ok(false) };
            store.transition(&t.id, State::Dropped, Some(&r), None)?;
            println!("  -> 取り下げ");
            Ok(true)
        }
        Some(2) => {
            let due = loop {
                let Some(d) = ui.text("新しい期限 (YYYY-MM-DD)") else { return Ok(false) };
                if parse_date(&d).is_ok() {
                    break d;
                }
                println!("  日付の形式が違います");
            };
            let Some(r) = ui.text("持ち越す理由") else { return Ok(false) };
            store.carry_over(&t.id, &r, to, Some(&due))?;
            println!("  -> 期限を {due} にして {to} に持ち越し（回数はリセット）");
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// Jev may label an unknown node. The line is a hint; only `revise` writes an edge.
fn print_jev_hints(store: &Store, date: &str) {
    let cfg = crate::config::load();
    if !cfg.jev.mode.eq_ignore_ascii_case("on") {
        return;
    }
    let key_ok = std::env::var("DAYLOOP_JEV_API_KEY")
        .map(|k| !k.trim().is_empty())
        .unwrap_or(false);
    if !key_ok || cfg.jev.route.trim().is_empty() {
        return;
    }
    let mut decider = crate::jev::HttpDecider {
        route: cfg.jev.route.clone(),
        timeout_ms: cfg.jev.timeout_ms,
    };
    let min = cfg.jev.commit_confidence.unwrap_or(0.0);
    let Ok(rows) = crate::jev::propose_unknown(store, date, &mut decider, min) else {
        return;
    };
    for row in rows {
        println!(
            "  新しい節: {} -> {} ({})",
            row.title, row.choice, row.confidence
        );
    }
    let tasks = match crate::order::day_tasks(store, date) {
        Ok(tasks) => tasks,
        Err(_) => return,
    };
    let spans = crate::order::spans_from_events(&store.events_for_day(date).unwrap_or_default());
    if let Some((a, b)) = crate::order::first_open_tie(date, &tasks, &spans) {
        if let Some(choice) = crate::order::tie_hint(&mut decider, &a.title, &b.title) {
            let other = if choice == a.title { &b.title } else { &a.title };
            println!("  順番のヒント: 「{choice}」を先に。覚えるには prefer {choice} {other}");
        }
    }
}

fn option_labels(q: &Question) -> Vec<&str> {
    q.options.iter().map(|o| o.label.as_str()).collect()
}

fn apply_observations(store: &Store, date: &str) -> Result<()> {
    let cfg = crate::config::load();
    let map = crate::observe::load_map(&cfg.observe)?;
    let now = chrono::Local::now().format("%H:%M").to_string();
    crate::observe::apply_day(store, date, &map, &now)?;
    Ok(())
}

/// Evening: every open task becomes done / not_done / carried / dropped, then the day closes.
pub fn close_ritual(store: &Store, ui: &Ui, date: &str) -> Result<Outcome> {
    apply_observations(store, date)?;
    graph::apply_known_tasks(store, date)?;
    print_jev_hints(store, date);
    let floor = crate::config::load().jev.commit_confidence;
    if floor.is_some() {
        let cfg = crate::config::load();
        let mut decider = crate::jev::HttpDecider {
            route: cfg.jev.route.clone(),
            timeout_ms: cfg.jev.timeout_ms,
        };
        crate::jev::apply_if_measured(store, date, &mut decider, floor)?;
    }
    let open = crate::order::day_tasks(store, date)?
        .into_iter()
        .filter(|t| t.state.is_open())
        .collect::<Vec<_>>();
    println!("== {date} のクローズ: 未確定 {} 件", open.len());
    let mut pending = 0usize;
    for t in open {
        let q = Question::close_task(&t);
        let labels = option_labels(&q);
        match ui.choose(&q.question, &labels) {
            None => pending += 1,
            Some(0) => {
                store.transition(&t.id, State::Done, None, None)?;
                println!("  -> 完了");
            }
            Some(1) => match ui.text("未完了の理由") {
                Some(r) => {
                    store.transition(&t.id, State::NotDone, Some(&r), None)?;
                    println!("  -> 未完了");
                }
                None => pending += 1,
            },
            Some(2) => {
                if !carry_flow(store, ui, &t, date)? {
                    pending += 1;
                }
            }
            Some(_) => match ui.text("取り下げる理由") {
                Some(r) => {
                    store.transition(&t.id, State::Dropped, Some(&r), None)?;
                    println!("  -> 取り下げ");
                }
                None => pending += 1,
            },
        }
    }
    if pending > 0 {
        println!();
        println!("未確定 {pending} 件。全件確定するまで {date} は閉じられません。");
        return Ok(Outcome::Pending(pending));
    }
    match store.close_day(date)? {
        Ok(()) => {
            println!();
            println!("{date} を閉じました。");
            print_day(store, date)?;
            Ok(Outcome::Done)
        }
        Err(rest) => Ok(Outcome::Pending(rest.len())),
    }
}

/// Morning: close leftovers first, triage candidates, pull backlog, confirm today's plan.
pub fn plan_ritual(store: &Store, ui: &Ui, date: &str) -> Result<Outcome> {
    for d in store.unclosed_days_before(date)? {
        if store.open_tasks_for_day(&d)?.is_empty() {
            let _ = store.close_day(&d)?;
            continue;
        }
        println!("!! {d} が未確定のままです。先に確定します。");
        if let Outcome::Pending(n) = close_ritual(store, ui, &d)? {
            return Ok(Outcome::Pending(n));
        }
        println!();
    }

    apply_observations(store, date)?;
    graph::apply_known_candidates(store, date)?;
    graph::apply_known_tasks(store, date)?;

    let mut pending = 0usize;

    let mut cands = store.open_candidates()?;
    cands.sort_by_key(|c| std::cmp::Reverse(days_since(&c.created_at) >= CANDIDATE_STALE_DAYS));
    if !cands.is_empty() {
        println!("== 候補 {} 件", cands.len());
    }
    for c in cands {
        let q = Question::candidate(&c, date);
        let labels = option_labels(&q);
        match ui.choose(&q.question, &labels) {
            None => pending += 1,
            Some(0) => {
                store.accept_candidate(&c.id, Some(date))?;
                println!("  -> 今日の予定");
            }
            Some(1) => {
                store.accept_candidate(&c.id, None)?;
                println!("  -> 未計画");
            }
            Some(2) => {
                store.reject_candidate(&c.id)?;
                println!("  -> 却下");
            }
            Some(_) => {}
        }
    }

    let backlog = store.backlog()?;
    if !backlog.is_empty() {
        println!();
        println!("== 未計画 {} 件", backlog.len());
    }
    for t in backlog {
        let q = Question::backlog(&t, date);
        let labels = option_labels(&q);
        if let Some(0) = ui.choose(&q.question, &labels) {
            store.schedule(&t.id, date)?;
            println!("  -> 今日の予定");
        }
    }

    println!();
    print_day(store, date)?;
    if pending > 0 {
        println!();
        println!("未回答 {pending} 件。回答後に plan を再実行してください。");
        return Ok(Outcome::Pending(pending));
    }
    let n = store.open_tasks_for_day(date)?.len();
    let q = Question::confirm_plan(date, n);
    let labels = option_labels(&q);
    match ui.choose(&q.question, &labels) {
        Some(0) => {
            store.confirm_plan(date)?;
            println!("確定しました。");
            Ok(Outcome::Done)
        }
        _ => Ok(Outcome::Pending(1)),
    }
}

/// Midday: untouched tasks get a decision.
pub fn check_ritual(store: &Store, ui: &Ui, date: &str) -> Result<Outcome> {
    apply_observations(store, date)?;
    graph::apply_known_tasks(store, date)?;
    let open = crate::order::day_tasks(store, date)?
        .into_iter()
        .filter(|t| t.state.is_open())
        .collect::<Vec<_>>();
    let doing: Vec<&Task> = open.iter().filter(|t| t.state == State::InProgress).collect();
    let untouched: Vec<&Task> = open.iter().filter(|t| t.state == State::Planned).collect();
    println!("== {date} の途中確認: 未着手 {} 件 / 進行中 {} 件", untouched.len(), doing.len());
    for t in &doing {
        println!("   進行中  {}  {}{}", short(&t.id), t.title, meta(t));
    }
    let mut pending = 0usize;
    for t in untouched {
        let q = Question::check_task(t);
        let labels = option_labels(&q);
        match ui.choose(&q.question, &labels) {
            None => pending += 1,
            Some(0) => {}
            Some(1) => {
                store.transition(&t.id, State::InProgress, None, None)?;
                println!("  -> 進行中");
            }
            Some(2) => {
                if !carry_flow(store, ui, t, date)? {
                    pending += 1;
                }
            }
            Some(_) => match ui.text("取り下げる理由") {
                Some(r) => {
                    store.transition(&t.id, State::Dropped, Some(&r), None)?;
                    println!("  -> 取り下げ");
                }
                None => pending += 1,
            },
        }
    }
    if pending > 0 {
        return Ok(Outcome::Pending(pending));
    }
    Ok(Outcome::Done)
}

/// Weekly: numbers, repeat offenders, a note.
pub fn retro_ritual(store: &Store, ui: &Ui, date: &str) -> Result<Outcome> {
    let (from, to) = week_range(date)?;
    let tasks = store.tasks_in_range(&from, &to)?;
    let count = |s: State| tasks.iter().filter(|t| t.state == s).count();
    let done = count(State::Done);
    let not_done = count(State::NotDone);
    let carried = count(State::Carried);
    let dropped = count(State::Dropped);
    let open = tasks.iter().filter(|t| t.state.is_open()).count();
    let decided = done + not_done + carried + dropped;
    println!("== 振り返り {from} 〜 {to}");
    println!("   予定 {}  完了 {done}  未完了 {not_done}  持ち越し {carried}  取り下げ {dropped}  未確定 {open}", tasks.len());
    if decided > 0 {
        println!("   完了率 {}%（確定した {decided} 件のうち）", done * 100 / decided);
    }
    println!();
    println!("   日付         予定  完了  未完了  持越  取下");
    let mut d = parse_date(&from)?;
    let end = parse_date(&to)?;
    while d <= end {
        let ds = d.format("%Y-%m-%d").to_string();
        let day: Vec<&Task> = tasks.iter().filter(|t| t.plan_date.as_deref() == Some(&ds)).collect();
        if !day.is_empty() {
            let c = |s: State| day.iter().filter(|t| t.state == s).count();
            println!(
                "   {ds}  {:>4}  {:>4}  {:>6}  {:>4}  {:>4}",
                day.len(),
                c(State::Done),
                c(State::NotDone),
                c(State::Carried),
                c(State::Dropped)
            );
        }
        d += chrono::Duration::days(1);
    }
    let mut repeat: Vec<&Task> = tasks.iter().filter(|t| t.carried_count >= 2).collect();
    repeat.sort_by_key(|t| std::cmp::Reverse(t.carried_count));
    repeat.dedup_by(|a, b| a.title == b.title);
    if !repeat.is_empty() {
        println!();
        println!("   持ち越しが多いタスク:");
        for t in repeat.iter().take(5) {
            println!("     {}回  {}  [{}]", t.carried_count, t.title, t.state.label_ja());
        }
    }

    let mut pending = 0usize;
    let blocked = store.blocked_carry_tasks()?;
    if !blocked.is_empty() {
        println!();
        println!("== {MAX_CARRY} 回以上持ち越したタスク {} 件。方針を決めてください。", blocked.len());
    }
    for t in blocked {
        println!();
        println!("   {}  {}{}", short(&t.id), t.title, meta(&t));
        let to = next_workday(t.plan_date.as_deref().unwrap_or(date))?;
        if !resolve_blocked(store, ui, &t, &to)? {
            pending += 1;
        }
    }

    if let Some(note) = ui.line("振り返りメモ（空でも可）") {
        if !note.is_empty() {
            store.set_retro(&to, &note)?;
            println!("  -> 記録しました");
        }
    }
    if pending > 0 {
        return Ok(Outcome::Pending(pending));
    }
    Ok(Outcome::Done)
}
