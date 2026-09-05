//! The daily loop: plan (morning) → check (midday) → close (evening) → retro (weekly).
//! Every decision is made by the person; the code only refuses to move on while
//! something is still undecided.

use std::io::{self, BufRead, IsTerminal, Write};

use anyhow::Result;

use crate::model::{State, Task};
use crate::store::{CarryBlocked, Store, CANDIDATE_STALE_DAYS, MAX_CARRY};
use crate::util::{days_since, hhmm, next_workday, parse_date, short, week_range};

pub struct Ui {
    interactive: bool,
}

impl Ui {
    pub fn new(yes: bool) -> Ui {
        // DAYLOOP_INTERACTIVE=1 forces prompts even when stdin is piped (tests, wrappers).
        let forced = std::env::var("DAYLOOP_INTERACTIVE")
            .map(|v| v == "1")
            .unwrap_or(false);
        Ui {
            interactive: !yes && (forced || io::stdin().is_terminal()),
        }
    }

    /// One question, up to 4 options, first is the default. None = not answered.
    pub fn choose(&self, q: &str, opts: &[&str]) -> Option<usize> {
        println!();
        println!("? {q}");
        for (i, o) in opts.iter().enumerate() {
            println!(
                "  {}) {}{}",
                i + 1,
                o,
                if i == 0 { "  (Enter で既定)" } else { "" }
            );
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
    let tasks = store.tasks_for_day(date)?;
    let open = tasks.iter().filter(|t| t.state.is_open()).count();
    println!("== {date}");
    match &day {
        Some(d) if d.closed_at.is_some() => {
            println!(
                "   クローズ済み {}",
                hhmm(d.closed_at.as_deref().unwrap_or(""))
            )
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
            let loc = e
                .location
                .as_deref()
                .map(|l| format!("  {l}"))
                .unwrap_or_default();
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
            println!(
                "     {}  {}{}{}{}",
                short(&t.id),
                t.title,
                meta(t),
                overdue,
                reason
            );
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
            if stale > 0 {
                format!("（うち {stale} 件が {CANDIDATE_STALE_DAYS} 日以上放置）")
            } else {
                String::new()
            }
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
        println!(
            "  「{}」は既に {} 回持ち越されています。",
            t.title, t.carried_count
        );
        return resolve_blocked(store, ui, t, &to);
    }
    let Some(reason) = ui.text("持ち越す理由") else {
        return Ok(false);
    };
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
    match ui.choose(
        "どうしますか",
        &[
            "分割する",
            "取り下げる",
            "期限を変えて持ち越す",
            "後で決める",
        ],
    ) {
        Some(0) => {
            let mut titles = Vec::new();
            loop {
                let Some(s) = ui.line(&format!(
                    "分割後のタスク {}（空行で終了）",
                    titles.len() + 1
                )) else {
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
            let Some(r) = ui.text("取り下げる理由") else {
                return Ok(false);
            };
            store.transition(&t.id, State::Dropped, Some(&r), None)?;
            println!("  -> 取り下げ");
            Ok(true)
        }
        Some(2) => {
            let due = loop {
                let Some(d) = ui.text("新しい期限 (YYYY-MM-DD)") else {
                    return Ok(false);
                };
                if parse_date(&d).is_ok() {
                    break d;
                }
                println!("  日付の形式が違います");
            };
            let Some(r) = ui.text("持ち越す理由") else {
                return Ok(false);
            };
            store.carry_over(&t.id, &r, to, Some(&due))?;
            println!("  -> 期限を {due} にして {to} に持ち越し（回数はリセット）");
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// Evening: every open task becomes done / not_done / carried / dropped, then the day closes.
pub fn close_ritual(store: &Store, ui: &Ui, date: &str) -> Result<Outcome> {
    store.prepare_day(date)?;
    let open = store.open_tasks_for_day(date)?;
    println!("== {date} のクローズ: 未確定 {} 件", open.len());
    let mut pending = 0usize;
    for t in open {
        let q = format!("{}  {}{}", short(&t.id), t.title, meta(&t));
        match ui.choose(
            &q,
            &[
                "完了",
                "未完了（理由を書く）",
                "持ち越し（理由を書く）",
                "取り下げ（理由を書く）",
            ],
        ) {
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
    pending += review_ritual(store, ui, date)?;
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
    let prepared = store.prepare_day(date)?;
    for gap in &prepared.missed_routines {
        println!(
            "未生成の定期タスク: {} {}（自動完了しません）",
            gap.date, gap.title
        );
    }
    for d in store.unclosed_days_before(date)? {
        println!("!! {d} が未確定のままです。先に確定します。");
        if let Outcome::Pending(n) = close_ritual(store, ui, &d)? {
            return Ok(Outcome::Pending(n));
        }
        println!();
    }

    let mut pending = 0usize;

    let mut cands = store.open_candidates()?;
    cands.sort_by_key(|c| std::cmp::Reverse(days_since(&c.created_at) >= CANDIDATE_STALE_DAYS));
    if !cands.is_empty() {
        println!("== 候補 {} 件", cands.len());
    }
    for c in cands {
        let age = days_since(&c.created_at);
        let stale = if age >= CANDIDATE_STALE_DAYS {
            format!("  !! {age} 日放置")
        } else {
            String::new()
        };
        let q = format!("{}  {}  ({}){stale}", short(&c.id), c.title, c.source);
        match ui.choose(
            &q,
            &["今日の予定に入れる", "未計画に入れる", "却下", "後で決める"],
        ) {
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
        let q = format!("{}  {}{}", short(&t.id), t.title, meta(&t));
        if let Some(0) = ui.choose(&q, &["今日やる", "そのまま"]) {
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
    match ui.choose(
        &format!("{date} の予定 {n} 件をこの内容で確定しますか"),
        &["確定する", "まだ"],
    ) {
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
    store.prepare_day(date)?;
    let open = store.open_tasks_for_day(date)?;
    let doing: Vec<&Task> = open
        .iter()
        .filter(|t| t.state == State::InProgress)
        .collect();
    let untouched: Vec<&Task> = open.iter().filter(|t| t.state == State::Planned).collect();
    println!(
        "== {date} の途中確認: 未着手 {} 件 / 進行中 {} 件",
        untouched.len(),
        doing.len()
    );
    for t in &doing {
        println!("   進行中  {}  {}{}", short(&t.id), t.title, meta(t));
    }
    let mut pending = 0usize;
    for t in untouched {
        let q = format!("未着手  {}  {}{}", short(&t.id), t.title, meta(t));
        match ui.choose(
            &q,
            &["今日中にやる", "今から着手", "持ち越す", "取り下げる"],
        ) {
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
    pending += review_ritual(store, ui, date)?;
    if pending > 0 {
        return Ok(Outcome::Pending(pending));
    }
    Ok(Outcome::Done)
}

fn review_ritual(store: &Store, ui: &Ui, date: &str) -> Result<usize> {
    use crate::business::ReviewOutcome;
    for r in store.pending_reviews(date)? {
        match ui.choose(
            &format!("{} の {} を確認しましたか", date, r.category.label_ja()),
            &[
                "確認済み",
                "対応が必要（保存済みIDを指定）",
                "未確認（理由を残す）",
                "後で確認",
            ],
        ) {
            Some(0) => {
                store.record_review(
                    date,
                    r.category,
                    ReviewOutcome::Confirmed,
                    None,
                    None,
                    None,
                )?;
            }
            Some(1) => {
                if let Some(id) = ui.text("関連タスクまたは候補の完全ID") {
                    let kind = ui.choose("関連付ける種類", &["タスク", "候補"]);
                    match kind {
                        Some(0) => {
                            store.record_review(
                                date,
                                r.category,
                                ReviewOutcome::NeedsAction,
                                None,
                                Some(&id),
                                None,
                            )?;
                        }
                        Some(1) => {
                            store.record_review(
                                date,
                                r.category,
                                ReviewOutcome::NeedsAction,
                                None,
                                None,
                                Some(&id),
                            )?;
                        }
                        _ => {}
                    }
                }
            }
            Some(2) => {
                if let Some(reason) = ui.text("確認できなかった理由") {
                    store.record_review(
                        date,
                        r.category,
                        ReviewOutcome::NotChecked,
                        Some(&reason),
                        None,
                        None,
                    )?;
                }
            }
            _ => {}
        }
    }
    Ok(store.pending_reviews(date)?.len())
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
    if let Some(rate) = (done * 100).checked_div(decided) {
        println!("   完了率 {rate}%（確定した {decided} 件のうち）");
    }
    println!();
    println!("   日付         予定  完了  未完了  持越  取下");
    let mut d = parse_date(&from)?;
    let end = parse_date(&to)?;
    while d <= end {
        let ds = d.format("%Y-%m-%d").to_string();
        let day: Vec<&Task> = tasks
            .iter()
            .filter(|t| t.plan_date.as_deref() == Some(&ds))
            .collect();
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
            println!(
                "     {}回  {}  [{}]",
                t.carried_count,
                t.title,
                t.state.label_ja()
            );
        }
    }

    let mut pending = 0usize;
    let blocked = store.blocked_carry_tasks()?;
    if !blocked.is_empty() {
        println!();
        println!(
            "== {MAX_CARRY} 回以上持ち越したタスク {} 件。方針を決めてください。",
            blocked.len()
        );
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
            store.set_retro(date, &note)?;
            println!("  -> 記録しました");
        }
    }
    if pending > 0 {
        return Ok(Outcome::Pending(pending));
    }
    Ok(Outcome::Done)
}
