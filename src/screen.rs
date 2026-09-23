//! One-shot read of the Outlook or Teams window the user is looking at.
//! wincli is called only for `window get_foreground` and whole-window `ui read`.
//! Element ids are never stored. The body is not sent to Jev unless the user asks.

use anyhow::{bail, Result};
use rusqlite::params;

use crate::jev::{Decider, JevOutcome};
use crate::store::Store;
use crate::util::{now, short};

pub const QUESTION_VERSION: &str = "screen-v1";
const MODEL_SENT: &str = "jev-latest";
const MODEL_LOCAL: &str = "local";
const BODY_CAP: usize = 20_000;
const SEND_CAP: usize = 4_000;

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS screen_captures (
  id TEXT PRIMARY KEY,
  app TEXT NOT NULL,
  window_title TEXT NOT NULL,
  process_name TEXT,
  captured_at TEXT NOT NULL,
  body TEXT,
  method TEXT NOT NULL,
  status TEXT NOT NULL,
  failure TEXT,
  folded INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS screen_evals (
  id TEXT PRIMARY KEY,
  capture_id TEXT NOT NULL,
  question_version TEXT NOT NULL,
  model_id TEXT NOT NULL,
  evaluated_at TEXT NOT NULL,
  sent INTEGER NOT NULL,
  kind TEXT,
  relation TEXT,
  priority TEXT,
  link TEXT,
  next_action TEXT,
  confidence REAL,
  reason_codes TEXT,
  quote TEXT
);
CREATE TABLE IF NOT EXISTS screen_decisions (
  id TEXT PRIMARY KEY,
  capture_id TEXT NOT NULL,
  decided_at TEXT NOT NULL,
  action TEXT NOT NULL,
  before_title TEXT,
  after_title TEXT,
  candidate_id TEXT
);
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppKind {
    Outlook,
    Teams,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Foreground {
    pub handle: String,
    pub title: String,
    pub process_name: String,
    pub elevated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadText {
    pub ok: bool,
    pub text: String,
    pub method: String,
    pub error: String,
    pub error_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BodyStatus {
    Ok { folded: bool },
    Unusable,
}

#[derive(Debug)]
pub struct ReadFail {
    pub message: String,
}

pub trait ScreenReader {
    fn foreground_json(&mut self) -> std::result::Result<String, ReadFail>;
    fn read_json(&mut self, handle: &str) -> std::result::Result<String, ReadFail>;
}

struct CaptureRow {
    id: String,
    app: String,
    window_title: String,
    process_name: String,
    body: Option<String>,
    method: String,
    status: String,
    failure: Option<String>,
    folded: bool,
}

struct EvalRow {
    sent: bool,
    kind: Option<String>,
    relation: Option<String>,
    priority: Option<String>,
    link: Option<String>,
    next_action: Option<String>,
    confidence: Option<f64>,
    reason_codes: String,
    quote: Option<String>,
    model_id: String,
}

pub fn classify(process: &str, title: &str) -> AppKind {
    let p = process.trim().to_ascii_lowercase();
    let p = p.strip_suffix(".exe").unwrap_or(&p);
    match p {
        "olk" | "outlook" | "hxoutlook" => return AppKind::Outlook,
        "ms-teams" | "msteams" | "teams" => return AppKind::Teams,
        "" => {}
        _ => return AppKind::Other,
    }
    let t = title.to_ascii_lowercase();
    if t.contains("outlook") {
        AppKind::Outlook
    } else if t.contains("teams") {
        AppKind::Teams
    } else {
        AppKind::Other
    }
}

pub fn app_label(kind: &AppKind) -> &'static str {
    match kind {
        AppKind::Outlook => "Outlook",
        AppKind::Teams => "Teams",
        AppKind::Other => "その他",
    }
}

fn app_source(kind: &AppKind) -> &'static str {
    match kind {
        AppKind::Outlook => "outlook",
        AppKind::Teams => "teams",
        AppKind::Other => "screen",
    }
}

pub fn foreground_command() -> Vec<String> {
    vec!["window".into(), "get_foreground".into()]
}

/// Whole-window read. No element id, so this never clicks or types.
pub fn read_command(handle: &str) -> Result<Vec<String>> {
    if handle.is_empty() || handle == "0" || !handle.chars().all(|c| c.is_ascii_digit()) {
        bail!("窓の番号が不正です");
    }
    Ok(vec![
        "ui".into(),
        "read".into(),
        "--window".into(),
        handle.into(),
        "--language".into(),
        "ja-JP".into(),
    ])
}

pub fn parse_foreground(raw: &str) -> std::result::Result<Foreground, String> {
    let v: serde_json::Value = serde_json::from_str(json_slice(raw)).map_err(|_| "読み取り結果を解釈できません".to_string())?;
    let success = v.get("success").and_then(|x| x.as_bool()).or_else(|| v.get("ok").and_then(|x| x.as_bool()));
    if success == Some(false) {
        let err = v.get("error").and_then(|x| x.as_str()).unwrap_or("前面の窓が取れません");
        return Err(clip(err, 200));
    }
    let Some(w) = v.get("window") else {
        return Err("前面の窓が取れません".into());
    };
    let handle = w.get("handle").and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
    if handle.is_empty() {
        return Err("前面の窓が取れません".into());
    }
    Ok(Foreground {
        handle,
        title: w.get("title").and_then(|x| x.as_str()).unwrap_or("").trim().to_string(),
        process_name: w.get("processName").and_then(|x| x.as_str()).unwrap_or("").trim().to_string(),
        elevated: w.get("isElevated").and_then(|x| x.as_bool()).unwrap_or(false),
    })
}

pub fn parse_read(raw: &str) -> std::result::Result<ReadText, String> {
    let v: serde_json::Value = serde_json::from_str(json_slice(raw)).map_err(|_| "読み取り結果を解釈できません".to_string())?;
    let Some(ok) = v.get("success").and_then(|x| x.as_bool()).or_else(|| v.get("ok").and_then(|x| x.as_bool())) else {
        return Err("読み取り結果を解釈できません".into());
    };
    let hint = v.get("hint").and_then(|x| x.as_str()).unwrap_or("");
    let method = if hint.to_ascii_lowercase().contains("ocr") { "ocr" } else { "uia" };
    Ok(ReadText {
        ok,
        text: v.get("text").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        method: method.to_string(),
        error: v.get("error").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        error_type: v.get("errorType").and_then(|x| x.as_str()).unwrap_or("").to_string(),
    })
}

fn json_slice(raw: &str) -> &str {
    raw.lines().rev().find(|l| l.trim_start().starts_with('{')).map(str::trim).unwrap_or(raw)
}

const CHROME: &[&str] = &[
    "outlook",
    "microsoft outlook",
    "teams",
    "microsoft teams",
    "受信トレイ",
    "inbox",
    "新規メール",
    "新しいメール",
    "new mail",
    "検索",
    "search",
    "予定表",
    "calendar",
    "チャット",
    "chat",
    "通話",
    "calls",
    "会議",
    "ファイル",
    "files",
    "アクティビティ",
    "activity",
    "ホーム",
    "home",
    "送信",
    "send",
    "削除",
    "delete",
    "返信",
    "reply",
    "転送",
    "forward",
    "未読",
    "unread",
];

fn is_chrome_line(line: &str) -> bool {
    let lower = line.trim().to_ascii_lowercase();
    CHROME.iter().any(|c| lower == *c)
}

pub fn text_is_only_title(text: &str, title: &str) -> bool {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    !title.is_empty() && flat == title
}

pub fn body_status(text: &str) -> BodyStatus {
    let kept: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !is_chrome_line(l))
        .collect();
    let n = kept.join("").chars().filter(|c| !c.is_whitespace()).count();
    if n < 8 {
        BodyStatus::Unusable
    } else {
        BodyStatus::Ok { folded: is_folded(text) }
    }
}

fn is_folded(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("さらに表示") || lower.contains("続きを表示") || lower.contains("show more") || text.contains("折りたたみ")
}

pub fn deadline_quote(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if norm_digit(chars[i]).is_none() {
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        while i < chars.len() && norm_digit(chars[i]).is_some() {
            i += 1;
        }
        if i >= chars.len() || chars[i] != '月' {
            i = start + 1;
            continue;
        }
        i += 1;
        let day = i;
        while i < chars.len() && norm_digit(chars[i]).is_some() {
            i += 1;
        }
        if i == day || i >= chars.len() || chars[i] != '日' {
            i = start + 1;
            continue;
        }
        i += 1;
        let mut end = i;
        let mut k = i;
        while k < chars.len() && (chars[k] == ' ' || chars[k] == '　') {
            k += 1;
        }
        if k < chars.len() && norm_digit(chars[k]).is_some() {
            let t0 = k;
            while k < chars.len() && norm_digit(chars[k]).is_some() {
                k += 1;
            }
            if k < chars.len() && chars[k] == '時' {
                k += 1;
                if k < chars.len() && norm_digit(chars[k]).is_some() {
                    while k < chars.len() && norm_digit(chars[k]).is_some() {
                        k += 1;
                    }
                    if k < chars.len() && chars[k] == '分' {
                        k += 1;
                    }
                }
                end = k;
                let _ = t0;
            }
        }
        let mut m = end;
        while m < chars.len() && m < end + 3 && (chars[m] == ' ' || chars[m] == '　') {
            m += 1;
        }
        if chars.get(m) == Some(&'ま') && chars.get(m + 1) == Some(&'で') {
            end = m + 2;
        }
        return Some(chars[start..end].iter().collect());
    }
    None
}

fn norm_digit(c: char) -> Option<char> {
    if c.is_ascii_digit() {
        return Some(c);
    }
    let u = c as u32;
    if (0xFF10..=0xFF19).contains(&u) {
        return char::from_u32('0' as u32 + (u - 0xFF10));
    }
    None
}

pub fn candidate_title(body: &str) -> String {
    for line in body.lines() {
        let t = line.trim();
        if t.chars().count() >= 8 && !is_chrome_line(t) {
            return clip(t, 80);
        }
    }
    clip(body.trim(), 80)
}

fn clip(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

pub fn kind_choices() -> Vec<String> {
    vec!["request".into(), "share".into(), "deadline_change".into(), "done_report".into(), "unknown".into()]
}
pub fn relation_choices() -> Vec<String> {
    vec!["mine".into(), "others".into(), "needs_check".into()]
}
pub fn priority_choices() -> Vec<String> {
    vec!["urgent".into(), "today".into(), "normal".into(), "insufficient".into()]
}
pub fn link_choices() -> Vec<String> {
    vec!["new".into(), "update".into(), "change".into(), "duplicate".into(), "unknown".into()]
}
pub fn next_choices() -> Vec<String> {
    vec!["register".into(), "match".into(), "ask_person".into(), "record_only".into()]
}

fn held(quote: Option<String>, codes: Vec<&str>) -> EvalRow {
    EvalRow {
        sent: false,
        kind: None,
        relation: None,
        priority: None,
        link: None,
        next_action: None,
        confidence: None,
        reason_codes: codes.join(","),
        quote,
        model_id: MODEL_LOCAL.into(),
    }
}

fn judge(body: &str, folded: bool, send: bool, decider: Option<&mut dyn Decider>) -> EvalRow {
    let quote = deadline_quote(body);
    let mut codes: Vec<&str> = Vec::new();
    if quote.is_some() {
        codes.push("deadline_stated");
    }
    if folded {
        codes.push("folded");
    }
    if !send {
        codes.push("not_sent");
        return held(quote, codes);
    }
    let Some(decider) = decider else {
        codes.push("jev_unavailable");
        return held(quote, codes);
    };
    let sample = clip(body, SEND_CAP);
    let questions = [
        ("この文章は、対応の依頼、情報の共有、期限の変更、完了の報告のどれか。どれでもなければ unknown。", kind_choices(), "unknown"),
        ("自分の担当か。自分なら mine、他者なら others、文面だけでは分からなければ needs_check。担当を推測しない。", relation_choices(), "needs_check"),
        ("対応の優先度。今すぐ見るなら urgent、今日の予定にするなら today、通常なら normal、材料が足りなければ insufficient。", priority_choices(), "insufficient"),
        ("既存の仕事との関係。新しい仕事なら new、既存への追加なら update、内容の変更なら change、同じ仕事の重複なら duplicate、分からなければ unknown。件名だけでは同一視しない。", link_choices(), "unknown"),
        ("次の扱い。候補として残すなら register、既存と照合するなら match、本人に聞くなら ask_person、記録だけなら record_only。", next_choices(), "ask_person"),
    ];
    let mut picks = Vec::new();
    let mut confs = Vec::new();
    for (instructions, choices, escape) in questions {
        let state = format!("問: {instructions}\n文章:\n{sample}");
        match decider.decide(&state, &choices) {
            JevOutcome::Answer { choice, confidence } => {
                if choices.iter().any(|c| c == &choice) {
                    picks.push(choice);
                } else {
                    picks.push(escape.to_string());
                    if !codes.contains(&"unlisted") {
                        codes.push("unlisted");
                    }
                }
                confs.push(confidence);
            }
            JevOutcome::Unavailable => {
                codes.push("jev_unavailable");
                return held(quote, codes);
            }
        }
    }
    if picks[1] == "needs_check" {
        codes.push("owner_unknown");
    }
    if picks[0] == "unknown" {
        codes.push("unclear");
    }
    let confidence = confs.into_iter().fold(1.0_f64, f64::min);
    EvalRow {
        sent: true,
        kind: Some(picks[0].clone()),
        relation: Some(picks[1].clone()),
        priority: Some(picks[2].clone()),
        link: Some(picks[3].clone()),
        next_action: Some(picks[4].clone()),
        confidence: Some(confidence),
        reason_codes: codes.join(","),
        quote,
        model_id: MODEL_SENT.into(),
    }
}

pub fn capture(store: &Store, reader: &mut dyn ScreenReader, send: bool, decider: Option<&mut dyn Decider>) -> Result<String> {
    let fg_raw = match reader.foreground_json() {
        Ok(s) => s,
        Err(e) => return save_failure(store, "unknown", "", "", "none", &e.message),
    };
    let fg = match parse_foreground(&fg_raw) {
        Ok(fg) => fg,
        Err(reason) => return save_failure(store, "unknown", "", "", "none", &reason),
    };
    if fg.elevated {
        return save_failure(store, "unknown", &fg.title, &fg.process_name, "none", "管理者として動いている窓は読めません");
    }
    let kind = classify(&fg.process_name, &fg.title);
    if kind == AppKind::Other {
        return save_failure(
            store,
            "unknown",
            &fg.title,
            &fg.process_name,
            "none",
            "前面の窓は Outlook でも Teams でもありません",
        );
    }
    let read_raw = match reader.read_json(&fg.handle) {
        Ok(s) => s,
        Err(e) => {
            return save_failure(store, app_source(&kind), &fg.title, &fg.process_name, "none", &e.message);
        }
    };
    let read = match parse_read(&read_raw) {
        Ok(r) => r,
        Err(reason) => return save_failure(store, app_source(&kind), &fg.title, &fg.process_name, "none", &reason),
    };
    if !read.ok {
        let reason = match read.error_type.as_str() {
            "ElevatedTarget" | "SecureDesktopActive" => "管理者として動いている窓は読めません".into(),
            _ if !read.error.trim().is_empty() => clip(read.error.trim(), 200),
            _ => "本文が取れませんでした".into(),
        };
        return save_failure(store, app_source(&kind), &fg.title, &fg.process_name, "none", &reason);
    }
    let text = clip(read.text.trim(), BODY_CAP);
    if text.is_empty() {
        return save_failure(store, app_source(&kind), &fg.title, &fg.process_name, &read.method, "本文が空です");
    }
    if text_is_only_title(&text, &fg.title) {
        return save_failure(
            store,
            app_source(&kind),
            &fg.title,
            &fg.process_name,
            &read.method,
            "本文が取れませんでした",
        );
    }
    let folded = match body_status(&text) {
        BodyStatus::Unusable => {
            return save_failure(
                store,
                app_source(&kind),
                &fg.title,
                &fg.process_name,
                &read.method,
                "本文が取れませんでした",
            );
        }
        BodyStatus::Ok { folded } => folded,
    };
    let eval = judge(&text, folded, send, decider);
    let row = CaptureRow {
        id: ulid::Ulid::new().to_string(),
        app: app_source(&kind).into(),
        window_title: fg.title.clone(),
        process_name: fg.process_name.clone(),
        body: Some(text.clone()),
        method: read.method.clone(),
        status: "ok".into(),
        failure: None,
        folded,
    };
    insert_capture(store, &row)?;
    insert_eval(store, &row.id, &eval)?;
    Ok(render_ok(&row, &eval))
}

fn save_failure(store: &Store, app: &str, title: &str, process: &str, method: &str, reason: &str) -> Result<String> {
    let row = CaptureRow {
        id: ulid::Ulid::new().to_string(),
        app: app.into(),
        window_title: title.into(),
        process_name: process.into(),
        body: None,
        method: method.into(),
        status: "failed".into(),
        failure: Some(reason.into()),
        folded: false,
    };
    insert_capture(store, &row)?;
    Ok(render_failure(&row.id, reason))
}

pub fn render_failure(id: &str, reason: &str) -> String {
    let reason = reason.trim().trim_start_matches("取得失敗").trim().trim_start_matches(':').trim().trim_start_matches('：').trim();
    format!("取得失敗: {reason}\n記録: {}\n本文は残していません。", short(id))
}

fn render_ok(row: &CaptureRow, eval: &EvalRow) -> String {
    let body = row.body.as_deref().unwrap_or("");
    let title = candidate_title(body);
    let app = if row.app == "teams" { "Teams" } else { "Outlook" };
    let mut out = String::new();
    out.push_str("【画面から取り込んだ内容】\n");
    out.push_str(&format!("アプリ: {app}\n"));
    out.push_str(&format!("取得元: {}\n", row.window_title));
    out.push_str(&format!("原文: {}\n", clip(body, 400)));
    out.push_str(&format!("取得方法: {}\n", row.method));
    if row.folded {
        out.push_str("折りたたみの続きは取れていません。\n");
    }
    out.push_str("\n【タスク候補】\n");
    out.push_str(&title);
    out.push_str("\n\n【Jevの評価】\n");
    out.push_str(&render_eval(eval));
    out.push_str("\n【残す情報】\n");
    out.push_str(&format!("記録: {}\n", short(&row.id)));
    out.push_str("原文、取得日時、取得元、評価、これからの採用・修正・保留を並べて残します。\n");
    out.push_str(&format!("  dayloop capture accept {}\n", short(&row.id)));
    out.push_str(&format!("  dayloop capture revise {} --title \"...\"\n", short(&row.id)));
    out.push_str(&format!("  dayloop capture hold {}\n", short(&row.id)));
    out
}

fn render_eval(eval: &EvalRow) -> String {
    if !eval.sent {
        let mut s = if eval.reason_codes.split(',').any(|c| c == "jev_unavailable") {
            "評価は保留。Jev に繋がりませんでした。本文は手元に残しています。\n".to_string()
        } else {
            "評価は保留。本文は送っていません。送るときは dayloop capture --send\n".to_string()
        };
        if let Some(q) = &eval.quote {
            s.push_str(&format!("根拠: {q}\n"));
        }
        return s;
    }
    let mut s = String::new();
    s.push_str(&format!("内容の種類: {}\n", ja_kind(eval.kind.as_deref().unwrap_or("unknown"))));
    s.push_str(&format!("自分との関係: {}\n", ja_relation(eval.relation.as_deref().unwrap_or("needs_check"))));
    s.push_str(&format!("対応の優先度: {}\n", ja_priority(eval.priority.as_deref().unwrap_or("insufficient"))));
    s.push_str(&format!("既存タスクとの関係: {}\n", ja_link(eval.link.as_deref().unwrap_or("unknown"))));
    s.push_str(&format!("次の扱い: {}\n", ja_next(eval.next_action.as_deref().unwrap_or("ask_person"))));
    if let Some(c) = eval.confidence {
        s.push_str(&format!("確度: {c:.2}\n"));
    }
    s.push_str(&format!("設問: {}\n", QUESTION_VERSION));
    let labels: Vec<String> = eval.reason_codes.split(',').filter(|c| !c.is_empty()).map(ja_reason).collect();
    if !labels.is_empty() {
        s.push_str(&format!("理由コード: {}\n", labels.join("、")));
    }
    if let Some(q) = &eval.quote {
        s.push_str(&format!("根拠: {q}\n"));
    }
    s
}

fn ja_kind(c: &str) -> &str {
    match c {
        "request" => "対応依頼",
        "share" => "情報共有",
        "deadline_change" => "期限変更",
        "done_report" => "完了報告",
        _ => "判断不能",
    }
}
fn ja_relation(c: &str) -> &str {
    match c {
        "mine" => "自分が担当",
        "others" => "他者が担当",
        _ => "確認が必要",
    }
}
fn ja_priority(c: &str) -> &str {
    match c {
        "urgent" => "至急確認",
        "today" => "今日の計画候補",
        "normal" => "通常",
        _ => "情報不足",
    }
}
fn ja_link(c: &str) -> &str {
    match c {
        "new" => "新規",
        "update" => "既存への追加情報",
        "change" => "変更候補",
        "duplicate" => "重複候補",
        _ => "不明",
    }
}
fn ja_next(c: &str) -> &str {
    match c {
        "register" => "候補登録",
        "match" => "既存タスクと照合",
        "record_only" => "記録のみ",
        _ => "本人に確認",
    }
}
fn ja_reason(c: &str) -> String {
    match c {
        "deadline_stated" => "期限の明示あり".into(),
        "owner_unknown" => "担当者不明".into(),
        "unclear" => "判断不能".into(),
        "folded" => "折りたたみの続きは未取得".into(),
        "unlisted" => "選択肢の外".into(),
        "jev_unavailable" => "Jevに繋がらない".into(),
        "not_sent" => "未送信".into(),
        other => other.into(),
    }
}

fn insert_capture(store: &Store, row: &CaptureRow) -> Result<()> {
    store.connection().execute(
        "INSERT INTO screen_captures(id,app,window_title,process_name,captured_at,body,method,status,failure,folded)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            row.id,
            row.app,
            row.window_title,
            row.process_name,
            now(),
            row.body,
            row.method,
            row.status,
            row.failure,
            if row.folded { 1 } else { 0 }
        ],
    )?;
    Ok(())
}

fn insert_eval(store: &Store, capture_id: &str, eval: &EvalRow) -> Result<()> {
    store.connection().execute(
        "INSERT INTO screen_evals(id,capture_id,question_version,model_id,evaluated_at,sent,kind,relation,priority,link,next_action,confidence,reason_codes,quote)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
        params![
            ulid::Ulid::new().to_string(),
            capture_id,
            QUESTION_VERSION,
            eval.model_id,
            now(),
            if eval.sent { 1 } else { 0 },
            eval.kind,
            eval.relation,
            eval.priority,
            eval.link,
            eval.next_action,
            eval.confidence,
            eval.reason_codes,
            eval.quote
        ],
    )?;
    Ok(())
}

struct Saved {
    id: String,
    app: String,
    body: Option<String>,
    status: String,
}

fn find_capture(store: &Store, idish: &str) -> Result<Saved> {
    let up = idish.trim().to_uppercase();
    let mut st = store.connection().prepare(
        "SELECT id,app,body,status FROM screen_captures WHERE id=?1 OR id LIKE ?2 OR id LIKE ?3",
    )?;
    let rows: Vec<Saved> = st
        .query_map(params![up, format!("{up}%"), format!("%{up}")], |r| {
            Ok(Saved {
                id: r.get(0)?,
                app: r.get(1)?,
                body: r.get(2)?,
                status: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    match rows.len() {
        0 => bail!("取り込みが見つかりません: {idish}"),
        1 => Ok(rows.into_iter().next().unwrap()),
        n => bail!("{idish} に一致する取り込みが {n} 件あります"),
    }
}

fn current_title(store: &Store, saved: &Saved) -> Result<String> {
    let revised: Option<String> = store
        .connection()
        .query_row(
            "SELECT after_title FROM screen_decisions WHERE capture_id=?1 AND action='revise' AND after_title IS NOT NULL ORDER BY decided_at DESC LIMIT 1",
            params![saved.id],
            |r| r.get(0),
        )
        .ok();
    if let Some(t) = revised {
        if !t.trim().is_empty() {
            return Ok(t);
        }
    }
    Ok(candidate_title(saved.body.as_deref().unwrap_or("")))
}

fn source_ref(id: &str) -> String {
    format!("screen:{id}")
}

fn record_decision(store: &Store, capture_id: &str, action: &str, before: Option<&str>, after: Option<&str>, candidate_id: Option<&str>) -> Result<()> {
    store.connection().execute(
        "INSERT INTO screen_decisions(id,capture_id,decided_at,action,before_title,after_title,candidate_id) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![ulid::Ulid::new().to_string(), capture_id, now(), action, before, after, candidate_id],
    )?;
    Ok(())
}

pub fn accept(store: &Store, idish: &str) -> Result<String> {
    let saved = find_capture(store, idish)?;
    if saved.status != "ok" || saved.body.as_deref().unwrap_or("").trim().is_empty() {
        bail!("本文が無いので採用できません");
    }
    let title = current_title(store, &saved)?;
    let before = candidate_title(saved.body.as_deref().unwrap_or(""));
    let refer = source_ref(&saved.id);
    let added = store.add_candidate(&title, &saved.app, Some(&refer))?;
    let cand_id = if let Some(c) = &added {
        c.id.clone()
    } else {
        store
            .connection()
            .query_row("SELECT id FROM candidates WHERE source_ref=?1", params![refer], |r| r.get(0))?
    };
    record_decision(store, &saved.id, "accept", Some(&before), Some(&title), Some(&cand_id))?;
    Ok(format!(
        "採用: {}  {}\n今日の予定にはまだ入っていません。入れるときは dayloop candidates accept {}",
        short(&cand_id),
        title,
        short(&cand_id)
    ))
}

pub fn revise(store: &Store, idish: &str, title: &str) -> Result<String> {
    let saved = find_capture(store, idish)?;
    if saved.status != "ok" {
        bail!("本文が無いので修正できません");
    }
    let title = title.trim();
    if title.is_empty() {
        bail!("タイトルが空です");
    }
    let title = clip(title, 200);
    let before = current_title(store, &saved)?;
    let refer = source_ref(&saved.id);
    store.connection().execute(
        "UPDATE candidates SET title=?1 WHERE source_ref=?2 AND status='open'",
        params![title, refer],
    )?;
    record_decision(store, &saved.id, "revise", Some(&before), Some(&title), None)?;
    Ok(format!("修正: {}  「{title}」\n評価はそのまま残しています。", short(&saved.id)))
}

pub fn hold(store: &Store, idish: &str) -> Result<String> {
    let saved = find_capture(store, idish)?;
    record_decision(store, &saved.id, "hold", None, None, None)?;
    Ok(format!("保留: {}\n記録は消していません。", short(&saved.id)))
}

pub fn list(store: &Store) -> Result<String> {
    let mut st = store.connection().prepare(
        "SELECT id,status,app,window_title,failure FROM screen_captures ORDER BY captured_at DESC LIMIT 20",
    )?;
    let rows: Vec<(String, String, String, String, Option<String>)> = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
        .collect::<rusqlite::Result<_>>()?;
    if rows.is_empty() {
        return Ok("画面からの取り込みはありません".into());
    }
    let mut out = String::new();
    for (id, status, app, title, failure) in rows {
        let label = if status == "ok" { title } else { failure.unwrap_or_else(|| "取得失敗".into()) };
        out.push_str(&format!("{}  {status}  {app}  {label}\n", short(&id)));
    }
    Ok(out)
}

pub fn locate_wincli() -> Option<std::path::PathBuf> {
    candidate_exes().into_iter().find(|p| p.is_file())
}

fn candidate_exes() -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Ok(p) = std::env::var("DAYLOOP_WINCLI") {
        let p = std::path::PathBuf::from(p.trim());
        if !p.as_os_str().is_empty() {
            out.push(p);
        }
    }
    let bin = crate::paths::data_dir().join("bin");
    out.push(bin.join(wincli_name()));
    out.push(bin.join(mcp_name()));
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            out.push(dir.join(wincli_name()));
            out.push(dir.join(mcp_name()));
        }
    }
    if let Some(p) = which_file(wincli_name()) {
        out.push(p);
    }
    if let Some(p) = which_file(mcp_name()) {
        out.push(p);
    }
    out
}

fn wincli_name() -> &'static str {
    if cfg!(windows) { "wincli.exe" } else { "wincli" }
}

fn mcp_name() -> &'static str {
    if cfg!(windows) { "Sbroenne.WindowsMcp.exe" } else { "Sbroenne.WindowsMcp" }
}

fn which_file(name: &str) -> Option<std::path::PathBuf> {
    let out = std::process::Command::new(if cfg!(windows) { "where.exe" } else { "which" })
        .arg(name)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().find(|l| !l.trim().is_empty())?;
    let p = std::path::PathBuf::from(line.trim());
    if p.is_file() { Some(p) } else { None }
}

fn is_cli_exe(path: &std::path::Path) -> bool {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.eq_ignore_ascii_case("wincli"))
        .unwrap_or(false)
}

/// Arguments for one whole-window read. No element id.
pub fn mcp_read_arguments(handle: &str) -> serde_json::Value {
    serde_json::json!({ "windowHandle": handle, "language": "ja-JP" })
}

pub struct WincliReader;

impl ScreenReader for WincliReader {
    fn foreground_json(&mut self) -> std::result::Result<String, ReadFail> {
        let fg = native_foreground()?;
        Ok(serde_json::json!({
            "success": true,
            "window": {
                "handle": fg.handle,
                "title": fg.title,
                "processName": fg.process_name,
                "isElevated": fg.elevated
            }
        })
        .to_string())
    }
    fn read_json(&mut self, handle: &str) -> std::result::Result<String, ReadFail> {
        let Some(exe) = locate_wincli() else {
            return Err(ReadFail { message: "wincli が見つかりません".into() });
        };
        if is_cli_exe(&exe) {
            let args = read_command(handle).map_err(|e| ReadFail { message: e.to_string() })?;
            return run_wincli(&exe, &args);
        }
        mcp_ui_read(&exe, handle)
    }
}

#[cfg(windows)]
fn native_foreground() -> std::result::Result<Foreground, ReadFail> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId};

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return Err(ReadFail { message: "前面の窓が取れません".into() });
        }
        let mut title = [0u16; 512];
        let n = GetWindowTextW(hwnd, &mut title);
        let title = String::from_utf16_lossy(&title[..n as usize]);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let mut process_name = String::new();
        if pid != 0 {
            if let Ok(proc) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                let mut buf = [0u16; 1024];
                let mut len = buf.len() as u32;
                if QueryFullProcessImageNameW(proc, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len).is_ok() {
                    let path = String::from_utf16_lossy(&buf[..len as usize]);
                    if let Some(file) = std::path::Path::new(&path).file_name().and_then(|s| s.to_str()) {
                        process_name = file.to_string();
                    }
                }
                let _ = CloseHandle(proc);
            }
        }
        Ok(Foreground {
            handle: (hwnd.0 as usize).to_string(),
            title,
            process_name,
            elevated: false,
        })
    }
}

#[cfg(not(windows))]
fn native_foreground() -> std::result::Result<Foreground, ReadFail> {
    Err(ReadFail { message: "この OS では画面を読めません".into() })
}

fn mcp_ui_read(exe: &std::path::Path, handle: &str) -> std::result::Result<String, ReadFail> {
    use std::io::{BufRead, Write};
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use std::time::Duration;

    let mut child = Command::new(exe)
        .args(["--tools", "ui_read"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| ReadFail { message: format!("wincli を起動できません: {e}") })?;
    let mut stdin = child.stdin.take().ok_or_else(|| ReadFail { message: "wincli を起動できません".into() })?;
    let stdout = child.stdout.take().ok_or_else(|| ReadFail { message: "wincli を起動できません".into() })?;
    let (tx, rx) = mpsc::channel::<std::result::Result<String, String>>();
    std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(stdout);
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => {
                    let _ = tx.send(Err("画面の読み取りが終了しました".into()));
                    break;
                }
                Ok(_) => {
                    if tx.send(Ok(line)).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e.to_string()));
                    break;
                }
            }
        }
    });
    let fail = |child: &mut std::process::Child, message: String| -> std::result::Result<String, ReadFail> {
        let _ = child.kill();
        let _ = child.wait();
        Err(ReadFail { message })
    };
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"dayloop","version":"0.1.0"}}}"#;
    if writeln!(stdin, "{init}").is_err() {
        return fail(&mut child, "wincli を起動できません".into());
    }
    let _ = stdin.flush();
    if let Err(message) = wait_mcp(&rx, 1, Duration::from_secs(20)) {
        return fail(&mut child, message);
    }
    let _ = writeln!(stdin, r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#);
    let call = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": { "name": "ui_read", "arguments": mcp_read_arguments(handle) }
    });
    if writeln!(stdin, "{call}").is_err() {
        return fail(&mut child, "wincli を起動できません".into());
    }
    let _ = stdin.flush();
    let line = match wait_mcp(&rx, 2, Duration::from_secs(45)) {
        Ok(line) => line,
        Err(message) => return fail(&mut child, message),
    };
    let _ = child.kill();
    let _ = child.wait();
    extract_tool_json(&line)
}

fn wait_mcp(rx: &std::sync::mpsc::Receiver<std::result::Result<String, String>>, id: i64, budget: std::time::Duration) -> std::result::Result<String, String> {
    let deadline = std::time::Instant::now() + budget;
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return Err("画面の読み取りが時間内に返りませんでした".into());
        }
        match rx.recv_timeout(left) {
            Ok(Ok(line)) => {
                let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) else { continue };
                if v.get("id").and_then(|x| x.as_i64()) == Some(id) {
                    return Ok(line);
                }
            }
            Ok(Err(message)) => return Err(message),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                return Err("画面の読み取りが時間内に返りませんでした".into());
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err("画面の読み取りが終了しました".into());
            }
        }
    }
}

fn extract_tool_json(line: &str) -> std::result::Result<String, ReadFail> {
    let v: serde_json::Value = serde_json::from_str(line.trim()).map_err(|_| ReadFail { message: "読み取り結果を解釈できません".into() })?;
    if let Some(err) = v.get("error") {
        let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("読み取り結果を解釈できません");
        return Err(ReadFail { message: clip(msg, 200) });
    }
    let text = v.pointer("/result/content/0/text").and_then(|t| t.as_str()).unwrap_or("");
    if text.trim_start().starts_with('{') {
        return Ok(text.to_string());
    }
    if text.trim().is_empty() {
        return Err(ReadFail { message: "wincli が文章を返しませんでした".into() });
    }
    Err(ReadFail { message: clip(text.trim(), 200) })
}

fn run_wincli(exe: &std::path::Path, args: &[String]) -> std::result::Result<String, ReadFail> {
    let out = std::process::Command::new(exe).args(args).output().map_err(|e| ReadFail {
        message: format!("wincli を起動できません: {e}"),
    })?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let body = json_slice(&stdout).trim().to_string();
    if body.starts_with('{') {
        return Ok(body);
    }
    let msg = if !stderr.trim().is_empty() { stderr.trim() } else { stdout.trim() };
    let msg = if msg.is_empty() { "wincli が文章を返しませんでした" } else { msg };
    Err(ReadFail { message: clip(msg, 200) })
}

pub fn run_foreground(store: &Store, send: bool) -> Result<String> {
    let mut reader = WincliReader;
    if send {
        if !crate::jev::ready(&crate::config::load().jev.mode, &crate::config::load().jev.route) {
            return capture(store, &mut reader, true, None);
        }
        let cfg = crate::config::load();
        let mut decider = crate::jev::HttpDecider {
            route: crate::jev::route_of(&cfg.jev.route),
            timeout_ms: cfg.jev.timeout_ms,
        };
        capture(store, &mut reader, true, Some(&mut decider))
    } else {
        capture(store, &mut reader, false, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed {
        fg: String,
        read: String,
        read_calls: usize,
    }
    impl ScreenReader for Fixed {
        fn foreground_json(&mut self) -> std::result::Result<String, ReadFail> {
            Ok(self.fg.clone())
        }
        fn read_json(&mut self, _handle: &str) -> std::result::Result<String, ReadFail> {
            self.read_calls += 1;
            Ok(self.read.clone())
        }
    }

    struct PanicReader;
    impl ScreenReader for PanicReader {
        fn foreground_json(&mut self) -> std::result::Result<String, ReadFail> {
            Err(ReadFail { message: "wincli が見つかりません".into() })
        }
        fn read_json(&mut self, _: &str) -> std::result::Result<String, ReadFail> {
            panic!("must not read");
        }
    }

    struct Scripted {
        answers: Vec<JevOutcome>,
        calls: usize,
    }
    impl Decider for Scripted {
        fn decide(&mut self, state: &str, choices: &[String]) -> JevOutcome {
            assert!(state.contains("文章"));
            assert!(choices.iter().any(|c| c == "unknown" || c == "needs_check" || c == "insufficient" || c == "ask_person"));
            self.calls += 1;
            self.answers.remove(0)
        }
    }

    fn fg(process: &str, title: &str, elevated: bool) -> String {
        serde_json::json!({
            "success": true,
            "window": {"handle":"42","title": title, "processName": process, "isElevated": elevated}
        })
        .to_string()
    }
    fn read_ok(text: &str, hint: &str) -> String {
        serde_json::json!({"success": true, "action": "ui_read", "text": text, "hint": hint}).to_string()
    }

    fn temp_store() -> (tempfile_dir::Guard, Store) {
        let _lock = LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("dayloop-screen-{}", ulid::Ulid::new()));
        std::fs::create_dir_all(&dir).unwrap();
        let prev = std::env::var("DAYLOOP_HOME").ok();
        std::env::set_var("DAYLOOP_HOME", &dir);
        let store = Store::open().unwrap();
        (tempfile_dir::Guard { dir, prev, _lock }, store)
    }

    #[test]
    fn classifies_new_outlook_and_teams_and_rejects_the_browser() {
        assert_eq!(classify("olk.exe", "受信トレイ"), AppKind::Outlook);
        assert_eq!(classify("ms-teams.exe", "チャット"), AppKind::Teams);
        assert_eq!(classify("msedge.exe", "Outlook"), AppKind::Other);
        assert_eq!(classify("", "メール - Outlook"), AppKind::Outlook);
    }

    #[test]
    fn read_command_is_whole_window_only() {
        let args = read_command("42").unwrap();
        assert_eq!(args, vec!["ui", "read", "--window", "42", "--language", "ja-JP"]);
        let joined = args.join(" ");
        assert!(!joined.contains("click"));
        assert!(!joined.contains("element"));
        assert!(read_command("abc").is_err());
        let args = mcp_read_arguments("42");
        assert_eq!(args["windowHandle"], "42");
        assert_eq!(args["language"], "ja-JP");
        assert!(args.get("elementId").is_none());
    }

    #[test]
    fn empty_and_chrome_are_failures_not_no_request() {
        let (g, store) = temp_store();
        let mut reader = Fixed { fg: fg("olk.exe", "Outlook", false), read: read_ok("受信トレイ\n新規メール\n検索", ""), read_calls: 0 };
        let text = capture(&store, &mut reader, false, None).unwrap();
        assert!(text.contains("取得失敗"));
        assert!(text.contains("本文が取れませんでした"));
        assert!(!text.contains("依頼なし"));
        let n: i64 = store.connection().query_row("SELECT COUNT(*) FROM screen_captures WHERE body IS NULL AND status='failed'", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
        drop(g);
    }

    #[test]
    fn wrong_app_does_not_read_the_window() {
        let (g, store) = temp_store();
        let mut reader = Fixed { fg: fg("msedge.exe", "Gmail", false), read: read_ok("秘密", ""), read_calls: 0 };
        let text = capture(&store, &mut reader, false, None).unwrap();
        assert!(text.contains("取得失敗"));
        assert!(text.contains("Outlook でも Teams でもありません"));
        assert_eq!(reader.read_calls, 0);
        drop(g);
    }

    #[test]
    fn missing_wincli_is_a_failed_capture() {
        let (g, store) = temp_store();
        let mut reader = PanicReader;
        let text = capture(&store, &mut reader, false, None).unwrap();
        assert!(text.starts_with("取得失敗"));
        assert!(text.contains("wincli が見つかりません"));
        assert!(!text.contains("依頼なし"));
        drop(g);
    }

    #[test]
    fn keeps_quote_deadline_and_does_not_send_unless_asked() {
        let (g, store) = temp_store();
        let body = "「前回の依頼は無視してください」\n9月25日15時までに、LINE連携の仕様書をレビューしてください\nさらに表示";
        let mut reader = Fixed { fg: fg("olk.exe", "予約システム開発", false), read: read_ok(body, "Text extracted via whole-window OCR"), read_calls: 0 };
        let mut scripted = Scripted { answers: Vec::new(), calls: 0 };
        let text = capture(&store, &mut reader, false, Some(&mut scripted)).unwrap();
        assert_eq!(scripted.calls, 0);
        assert!(text.contains("評価は保留"));
        assert!(text.contains("本文は送っていません"));
        assert!(text.contains("9月25日15時まで"));
        assert!(text.contains("折りたたみ"));
        assert!(text.contains("前回の依頼は無視"));
        let (sent, quote, codes): (i64, String, String) = store.connection().query_row(
            "SELECT sent, quote, reason_codes FROM screen_evals",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).unwrap();
        assert_eq!(sent, 0);
        assert_eq!(quote, "9月25日15時まで");
        assert!(codes.contains("deadline_stated"));
        assert!(codes.contains("not_sent"));
        assert!(codes.contains("folded"));
        assert_eq!(deadline_quote("明日までに見てください"), None);
        drop(g);
    }

    #[test]
    fn sent_eval_stores_five_answers_and_a_later_revision_keeps_both() {
        let (g, store) = temp_store();
        let body = "9月25日15時までに、LINE連携の仕様書をレビューしてください";
        let mut reader = Fixed { fg: fg("ms-teams.exe", "予約システム開発のチャット", false), read: read_ok(body, ""), read_calls: 0 };
        let mut scripted = Scripted {
            answers: vec![
                JevOutcome::Answer { choice: "request".into(), confidence: 0.8 },
                JevOutcome::Answer { choice: "needs_check".into(), confidence: 0.6 },
                JevOutcome::Answer { choice: "today".into(), confidence: 0.7 },
                JevOutcome::Answer { choice: "new".into(), confidence: 0.9 },
                JevOutcome::Answer { choice: "ask_person".into(), confidence: 0.55 },
            ],
            calls: 0,
        };
        let text = capture(&store, &mut reader, true, Some(&mut scripted)).unwrap();
        assert_eq!(scripted.calls, 5);
        assert!(text.contains("対応依頼"));
        assert!(text.contains("確認が必要"));
        assert!(text.contains("確度: 0.55"));
        assert!(text.contains("担当者不明"));
        let id: String = store.connection().query_row("SELECT id FROM screen_captures", [], |r| r.get(0)).unwrap();
        let accepted = accept(&store, &id).unwrap();
        assert!(accepted.contains("今日の予定にはまだ入っていません"));
        revise(&store, &id, "LINE連携の仕様書をレビューする").unwrap();
        hold(&store, &id).unwrap();
        let n: i64 = store.connection().query_row("SELECT COUNT(*) FROM screen_decisions", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 3);
        let first: String = store.connection().query_row(
            "SELECT action FROM screen_decisions ORDER BY decided_at ASC LIMIT 1",
            [],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(first, "accept");
        let evals: i64 = store.connection().query_row("SELECT COUNT(*) FROM screen_evals WHERE sent=1 AND kind='request'", [], |r| r.get(0)).unwrap();
        assert_eq!(evals, 1);
        let cand: String = store.connection().query_row("SELECT title FROM candidates", [], |r| r.get(0)).unwrap();
        assert_eq!(cand, "LINE連携の仕様書をレビューする");
        let refer: String = store.connection().query_row("SELECT source_ref FROM candidates", [], |r| r.get(0)).unwrap();
        assert!(refer.starts_with("screen:"));
        assert!(!refer.contains("element"));
        drop(g);
    }

    #[test]
    fn window_title_alone_is_a_failed_read() {
        let (g, store) = temp_store();
        let title = "一般 | Microsoft Teams";
        let mut reader = Fixed {
            fg: fg("ms-teams.exe", title, false),
            read: read_ok(title, ""),
            read_calls: 0,
        };
        let text = capture(&store, &mut reader, false, None).unwrap();
        assert!(text.contains("取得失敗"));
        assert!(text.contains("本文が取れませんでした"));
        assert!(!text.contains("依頼なし"));
        drop(g);
    }

    #[test]
    fn tool_error_is_failure() {
        let (g, store) = temp_store();
        let bad = r#"{"success":false,"errorType":"ElevatedTarget","error":"elevated"}"#;
        let mut reader = Fixed { fg: fg("outlook.exe", "Outlook", false), read: bad.into(), read_calls: 0 };
        let text = capture(&store, &mut reader, false, None).unwrap();
        assert!(text.contains("取得失敗"));
        assert!(text.contains("管理者として動いている窓は読めません"));
        assert!(!text.contains("依頼なし"));
        drop(g);
    }
}

#[cfg(test)]
static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tempfile_dir {
    pub struct Guard {
        pub dir: std::path::PathBuf,
        pub prev: Option<String>,
        pub _lock: std::sync::MutexGuard<'static, ()>,
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => std::env::set_var("DAYLOOP_HOME", v),
                None => std::env::remove_var("DAYLOOP_HOME"),
            }
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}
