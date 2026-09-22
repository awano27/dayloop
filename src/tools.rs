//! MCP tool implementations. Transport-agnostic: take JSON args, return JSON.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::engine;
use crate::markdown;
use crate::model::State;
use crate::store::{CarryBlocked, Store};
use crate::util::{next_workday, parse_date, resolve_date};

pub fn list() -> Vec<Value> {
    vec![
        tool("get_today", "今日（または指定日）のタスクと Day の状態、未処理候補数、未クローズの過去日を返す。会話の最初に呼ぶ。", obj(&[("date", str_date())], &[])),
        tool("plan_day", "朝に呼ぶ。未クローズ日・候補・未計画・今日の予定と questions を返す。本人に1問ずつ聞き、options の tool と needs で反映してから confirm_plan する。", obj(&[("date", str_date())], &[])),
        tool("confirm_plan", "plan_day の questions を処理したあと、その日の予定を確定し plan_confirmed_at を記録する。", obj(&[("date", str_date())], &[])),
        tool("add_task", "タスクを追加する。backlog=true なら未計画、そうでなければ date（省略時は今日）の予定になる。", obj(&[("title", schema("string", "タスク名")), ("due", schema("string", "期限 YYYY-MM-DD")), ("estimate_min", schema("integer", "見積（分）")), ("date", str_date()), ("backlog", schema("boolean", "未計画に入れる"))], &["title"])),
        tool("schedule_task", "未計画のタスクを指定日の予定に入れる。plan_day の backlog 質問への回答。", obj(&[("id", str_id()), ("date", schema("string", "予定日 YYYY-MM-DD"))], &["id", "date"])),
        tool("start_task", "タスクを進行中にする。未計画なら date（省略時は今日）の予定に入れてから着手する。", obj(&[("id", str_id()), ("date", str_date())], &["id"])),
        tool("finish_task", "タスクを完了にする。close_day の「完了」回答。", obj(&[("id", str_id()), ("evidence", schema("string", "完了の根拠（PR URL など）"))], &["id"])),
        tool("set_not_done", "タスクを未完了にする。reason は空にできない。", obj(&[("id", str_id()), ("reason", schema("string", "未完了の理由"))], &["id", "reason"])),
        tool("carry_over", "タスクを持ち越す。3回目以降は error=carry_blocked と question を返す（例外にしない）。期限変更は reschedule_due を付ける。", obj(&[("id", str_id()), ("reason", schema("string", "持ち越し理由")), ("to", schema("string", "持ち越し先 YYYY-MM-DD")), ("reschedule_due", schema("string", "新しい期限。付けると持ち越し回数をリセット"))], &["id", "reason"])),
        tool("drop_task", "タスクを取り下げる。reason は空にできない。", obj(&[("id", str_id()), ("reason", schema("string", "取り下げ理由"))], &["id", "reason"])),
        tool("split_task", "タスクを2つ以上に分割する。元は取り下げ、titles が to（省略時は次の営業日）の予定になる。持ち越し上限の解消に使う。", obj(&[("id", str_id()), ("titles", json!({"type":"array","items":{"type":"string"},"description":"分割後のタイトル（2つ以上）"})), ("reason", schema("string", "分割理由")), ("to", schema("string", "分割先 YYYY-MM-DD"))], &["id", "titles", "reason"])),
        tool("check_in", "昼に呼ぶ。進行中・未着手と questions を返す。本人に1問ずつ聞き、options の tool と needs で反映する。", obj(&[("date", str_date())], &[])),
        tool("close_day", "夕方に呼ぶ。open が残った場合は closed=false と questions を返す。既定値では閉じない。questions を本人に1問ずつ聞き、options の tool と needs で確定してから再度呼ぶ。", obj(&[("date", str_date())], &[])),
        tool("retro_week", "週末に呼ぶ。週の集計・日別・持ち越し上位と、carried_count>=3 の open タスクの questions を返す。", obj(&[("date", str_date())], &[])),
        tool("list_candidates", "採用/却下待ちの候補を返す。age_days と stale（7日以上）付き。", obj(&[], &[])),
        tool("add_candidate", "候補箱に追加する。同じ source_ref が既にあるか却下済みなら skipped を返す。", obj(&[("title", schema("string", "候補のタイトル")), ("source", schema("string", "teams / outlook / meeting / alert / manual")), ("source_ref", schema("string", "元メッセージ等への参照"))], &["title", "source"])),
        tool("accept_candidate", "候補をタスクにする。backlog=true なら未計画、そうでなければ date（省略時は今日）の予定。", obj(&[("id", str_id()), ("date", str_date()), ("backlog", schema("boolean", "未計画に入れる"))], &["id"])),
        tool("reject_candidate", "候補を却下する。同じ source_ref は再提示されない。", obj(&[("id", str_id())], &["id"])),
        tool("export_markdown", "指定日の Markdown を書き出し、パスを返す。", obj(&[("date", str_date())], &[])),
        tool("import_markdown", "指定日の Markdown の手編集を取り込む。[x] は完了、新しい - [ ] は追加。", obj(&[("date", str_date())], &[])),
        tool("sync_sources", "有効な取り込みアダプタを実行する。Outlook が無い環境でも部分失敗を返し、isError にはしない。", obj(&[], &[])),
        tool(
            "prefer_order",
            "同順位の2件について、先にやるタイトルを覚える。",
            obj(
                &[
                    ("first", schema("string", "先にやるタイトル")),
                    ("second", schema("string", "あとでやるタイトル")),
                ],
                &["first", "second"],
            ),
        ),
    ]
}

pub fn dispatch(store: &Store, name: &str, args: &Value) -> Value {
    match call(store, name, args) {
        Ok(v) => v,
        Err(e) => {
            if let Some(cb) = e.downcast_ref::<CarryBlocked>() {
                return json!({
                    "error": "carry_blocked",
                    "question": engine::Question::carry_blocked(&cb.task),
                });
            }
            json!({ "error": e.to_string() })
        }
    }
}

fn call(store: &Store, name: &str, args: &Value) -> Result<Value> {
    match name {
        "get_today" => engine::today_view(store, &date_arg(args)?),
        "plan_day" => {
            let d = date_arg(args)?;
            observe_first(store, &d)?;
            engine::plan_view(store, &d)
        }
        "confirm_plan" => {
            let d = date_arg(args)?;
            store.confirm_plan(&d)?;
            markdown::export(store, &d)?;
            let day = store.get_day(&d)?;
            Ok(json!({ "plan_confirmed_at": day.and_then(|x| x.plan_confirmed_at) }))
        }
        "add_task" => {
            let title = req_str(args, "title")?;
            if let Some(d) = opt_str(args, "due") {
                parse_date(&d)?;
            }
            let backlog = opt_bool(args, "backlog");
            let plan = if backlog { None } else { Some(date_arg(args)?) };
            let t = store.add_task(
                &title,
                opt_str(args, "due").as_deref(),
                opt_i64(args, "estimate_min"),
                "manual",
                None,
                plan.as_deref(),
            )?;
            export_task(store, &t)?;
            Ok(serde_json::to_value(t)?)
        }
        "schedule_task" => {
            let id = req_str(args, "id")?;
            let date = req_str(args, "date")?;
            parse_date(&date)?;
            let t = store.schedule(&id, &date)?;
            export_task(store, &t)?;
            Ok(serde_json::to_value(t)?)
        }
        "start_task" => {
            let id = req_str(args, "id")?;
            let mut t = store.get_task(&id)?;
            if t.plan_date.is_none() {
                let d = date_arg(args)?;
                t = store.schedule(&t.id, &d)?;
            }
            let t = store.transition(&t.id, State::InProgress, None, None)?;
            export_task(store, &t)?;
            Ok(serde_json::to_value(t)?)
        }
        "finish_task" => {
            let id = req_str(args, "id")?;
            let t = store.transition(&id, State::Done, None, opt_str(args, "evidence").as_deref())?;
            export_task(store, &t)?;
            Ok(serde_json::to_value(t)?)
        }
        "set_not_done" => {
            let id = req_str(args, "id")?;
            let reason = req_str(args, "reason")?;
            if reason.trim().is_empty() {
                anyhow::bail!("未完了には理由が必須です");
            }
            let t = store.transition(&id, State::NotDone, Some(&reason), None)?;
            export_task(store, &t)?;
            Ok(serde_json::to_value(t)?)
        }
        "carry_over" => {
            let id = req_str(args, "id")?;
            let reason = req_str(args, "reason")?;
            let old = store.get_task(&id)?;
            let base = old.plan_date.clone().unwrap_or_else(crate::util::today);
            let to = match opt_str(args, "to") {
                Some(t) => {
                    parse_date(&t)?;
                    t
                }
                None => next_workday(&base)?,
            };
            if let Some(r) = opt_str(args, "reschedule_due") {
                parse_date(&r)?;
            }
            let t = store.carry_over(&old.id, &reason, &to, opt_str(args, "reschedule_due").as_deref())?;
            export_task(store, &old)?;
            export_task(store, &t)?;
            Ok(serde_json::to_value(t)?)
        }
        "drop_task" => {
            let id = req_str(args, "id")?;
            let reason = req_str(args, "reason")?;
            if reason.trim().is_empty() {
                anyhow::bail!("取り下げには理由が必須です");
            }
            let t = store.transition(&id, State::Dropped, Some(&reason), None)?;
            export_task(store, &t)?;
            Ok(serde_json::to_value(t)?)
        }
        "split_task" => {
            let id = req_str(args, "id")?;
            let reason = req_str(args, "reason")?;
            let titles = args
                .get("titles")
                .and_then(|v| v.as_array())
                .ok_or_else(|| anyhow!("titles が必要です"))?
                .iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>();
            let old = store.get_task(&id)?;
            let base = old.plan_date.clone().unwrap_or_else(crate::util::today);
            let to = match opt_str(args, "to") {
                Some(t) => {
                    parse_date(&t)?;
                    t
                }
                None => next_workday(&base)?,
            };
            let news = store.split(&old.id, &titles, &reason, &to)?;
            export_task(store, &old)?;
            markdown::export(store, &to)?;
            Ok(serde_json::to_value(news)?)
        }
        "check_in" => {
            let d = date_arg(args)?;
            observe_first(store, &d)?;
            engine::check_view(store, &d)
        }
        "close_day" => {
            let d = date_arg(args)?;
            observe_first(store, &d)?;
            let open = store.open_tasks_for_day(&d)?;
            if !open.is_empty() {
                return engine::close_view(store, &d);
            }
            match store.close_day(&d)? {
                Ok(()) => {
                    markdown::export(store, &d)?;
                    Ok(json!({ "closed": true, "open": [], "questions": [] }))
                }
                Err(rest) => Ok(json!({
                    "closed": false,
                    "open": rest,
                    "questions": rest.iter().map(engine::Question::close_task).collect::<Vec<_>>(),
                })),
            }
        }
        "retro_week" => engine::retro_view(store, &date_arg(args)?),
        "list_candidates" => {
            let cands = store.open_candidates()?;
            Ok(json!(cands.iter().map(engine::candidate_json).collect::<Vec<_>>()))
        }
        "add_candidate" => {
            let title = req_str(args, "title")?;
            let source = req_str(args, "source")?;
            match store.add_candidate(&title, &source, opt_str(args, "source_ref").as_deref())? {
                Some(c) => Ok(engine::candidate_json(&c)),
                None => Ok(json!({ "skipped": "duplicate_or_rejected" })),
            }
        }
        "accept_candidate" => {
            let id = req_str(args, "id")?;
            let backlog = opt_bool(args, "backlog");
            let plan = if backlog { None } else { Some(date_arg(args)?) };
            let t = store.accept_candidate(&id, plan.as_deref())?;
            export_task(store, &t)?;
            Ok(serde_json::to_value(t)?)
        }
        "reject_candidate" => {
            let id = req_str(args, "id")?;
            store.reject_candidate(&id)?;
            Ok(json!({ "ok": true }))
        }
        "export_markdown" => {
            let d = date_arg(args)?;
            let p = markdown::export(store, &d)?;
            Ok(json!({ "path": p.to_string_lossy() }))
        }
        "import_markdown" => {
            let d = date_arg(args)?;
            let r = markdown::import(store, &d)?;
            markdown::export(store, &d)?;
            Ok(json!({ "completed": r.completed, "added": r.added }))
        }
        "sync_sources" => {
            let cfg = crate::config::load();
            Ok(crate::intake::sync_all_json(store, &cfg))
        }
        "prefer_order" => {
            let first = req_str(args, "first")?;
            let second = req_str(args, "second")?;
            crate::graph::record_order(store, &first, &second)?;
            Ok(json!({ "first": first, "second": second }))
        }
        _ => anyhow::bail!("unknown tool: {name}"),
    }
}

fn observe_first(store: &Store, date: &str) -> Result<()> {
    let cfg = crate::config::load();
    let map = crate::observe::load_map(&cfg.observe)?;
    let now = chrono::Local::now().format("%H:%M").to_string();
    crate::observe::apply_day(store, date, &map, &now)?;
    Ok(())
}

fn export_task(store: &Store, t: &crate::model::Task) -> Result<()> {
    if let Some(d) = &t.plan_date {
        markdown::export(store, d)?;
    }
    Ok(())
}

fn date_arg(args: &Value) -> Result<String> {
    resolve_date(opt_str(args, "date").as_deref())
}

fn req_str(args: &Value, k: &str) -> Result<String> {
    opt_str(args, k).ok_or_else(|| anyhow!("{k} が必要です"))
}

fn opt_str(args: &Value, k: &str) -> Option<String> {
    args.get(k).and_then(|v| {
        if v.is_null() {
            None
        } else {
            v.as_str().map(|s| s.to_string())
        }
    })
    .filter(|s| !s.is_empty())
}

fn opt_i64(args: &Value, k: &str) -> Option<i64> {
    args.get(k).and_then(|v| v.as_i64().or_else(|| v.as_str()?.parse().ok()))
}

fn opt_bool(args: &Value, k: &str) -> bool {
    args.get(k)
        .and_then(|v| v.as_bool().or_else(|| v.as_str().map(|s| s == "true" || s == "1")))
        .unwrap_or(false)
}

fn tool(name: &str, description: &str, input_schema: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema,
    })
}

fn obj(props: &[(&str, Value)], required: &[&str]) -> Value {
    let mut map = serde_json::Map::new();
    for (k, v) in props {
        map.insert((*k).to_string(), v.clone());
    }
    let mut schema = json!({
        "type": "object",
        "properties": map,
    });
    if !required.is_empty() {
        schema["required"] = json!(required);
    }
    schema
}

fn schema(ty: &str, description: &str) -> Value {
    json!({ "type": ty, "description": description })
}

fn str_date() -> Value {
    schema("string", "YYYY-MM-DD。省略時は今日")
}

fn str_id() -> Value {
    schema("string", "タスクまたは候補の ID（末尾8文字でも可）")
}
