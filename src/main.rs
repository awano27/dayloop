use anyhow::Result;
use clap::{Parser, Subcommand};

use dayloop::config;
use dayloop::doctor;
use dayloop::graph;
use dayloop::jev;
use dayloop::jev::Decider;
use dayloop::jev_eval;
use dayloop::order;
use dayloop::markdown;
use dayloop::mcp;
use dayloop::model;
use dayloop::model::State;
use dayloop::paths;
use dayloop::rituals::{self, Outcome, Ui};
use dayloop::serve;
use dayloop::startup;
use dayloop::store::{self, Store};
use dayloop::util::{self, next_workday, parse_date, resolve_date, short};

#[derive(Parser)]
#[command(
    name = "dayloop",
    version,
    about = "1日のタスクを 計画 → 確認 → 確定 → 振り返り で回す。忘れゼロは台帳で保証する。",
    after_help = "終了コード: 0 完了 / 1 エラー / 2 本人の回答待ち"
)]
struct Cli {
    /// 対話せずに実行する（回答が必要な項目は未回答のまま残し、終了コード 2）
    #[arg(long, global = true)]
    yes: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// 今日の状態を表示
    Today {
        #[arg(long)]
        date: Option<String>,
    },
    /// 次に手を付ける未完了タスクを1件出す
    Next {
        #[arg(long)]
        date: Option<String>,
    },
    /// 同順位の2件について、先にやるタイトルを覚える
    Prefer {
        first: String,
        second: String,
    },
    /// タスクを追加（既定は今日の予定）
    Add {
        title: String,
        /// 期限 YYYY-MM-DD
        #[arg(long)]
        due: Option<String>,
        /// 見積（分）
        #[arg(long)]
        estimate: Option<i64>,
        /// 予定日（既定は今日）
        #[arg(long)]
        date: Option<String>,
        /// 日付を決めず未計画に入れる
        #[arg(long)]
        backlog: bool,
    },
    /// 朝: 前日の未確定 → 候補 → 未計画 → 今日の予定を確定
    Plan {
        #[arg(long)]
        date: Option<String>,
    },
    /// 昼: 未着手タスクの確認
    Check {
        #[arg(long)]
        date: Option<String>,
    },
    /// 着手
    Start {
        id: String,
        /// 未計画のタスクを予定に入れて着手する日（既定は今日）
        #[arg(long)]
        date: Option<String>,
    },
    /// 完了
    Done {
        id: String,
        /// 完了の根拠（PR URL など）
        #[arg(long)]
        evidence: Option<String>,
    },
    /// 未完了（理由必須）
    Notdone {
        id: String,
        #[arg(long)]
        reason: String,
    },
    /// 持ち越し（理由必須。既定は次の営業日へ）
    Carry {
        id: String,
        #[arg(long)]
        reason: String,
        /// 持ち越し先 YYYY-MM-DD
        #[arg(long)]
        to: Option<String>,
        /// 期限を変更して持ち越す（持ち越し回数をリセット）
        #[arg(long)]
        reschedule: Option<String>,
    },
    /// 取り下げ（理由必須）
    Drop {
        id: String,
        #[arg(long)]
        reason: String,
    },
    /// 学習した枝を書き換え、開いている対象へすぐ適用する
    Revise {
        id: String,
        /// done | not_done | carry | drop | start | shelve | today | backlog | reject
        choice: String,
        #[arg(long)]
        reason: Option<String>,
    },
    /// 分割（元は取り下げ、分割先は次の営業日の予定）
    Split {
        id: String,
        #[arg(long)]
        reason: String,
        /// 分割後のタイトル（2つ以上）
        #[arg(long, required = true)]
        into: Vec<String>,
        /// 分割先の予定日（既定は次の営業日）
        #[arg(long)]
        to: Option<String>,
    },
    /// 夕: 全タスクを確定して日を閉じる
    Close {
        #[arg(long)]
        date: Option<String>,
    },
    /// 週次の振り返り（指定日を含む週）
    Retro {
        #[arg(long)]
        date: Option<String>,
    },
    /// 候補箱（外部から拾ったタスク候補）
    #[command(subcommand)]
    Candidates(CandCmd),
    /// 当日の Markdown を書き出す
    Export {
        #[arg(long)]
        date: Option<String>,
    },
    /// Markdown の手編集を取り込む
    Import {
        #[arg(long)]
        date: Option<String>,
    },
    /// この PC で何が使えるかを診断
    Doctor,
    /// データの場所を表示
    Where,
    /// MCP サーバー（stdio）。stdout は JSON-RPC 専用
    Mcp,
    /// 常駐し、設定した時刻に非対話で plan / check / close / retro を実行する
    Serve {
        /// コンソールウィンドウを出さない（ログオン時向け）
        #[arg(long)]
        quiet: bool,
    },
    /// ログオン時常駐の登録・解除
    #[command(subcommand)]
    Startup(StartupCmd),
    /// 設定ファイル
    #[command(subcommand)]
    Config(ConfigCmd),
    /// 外部ソースから Candidate / 予定を取り込む
    #[command(subcommand)]
    Intake(IntakeCmd),
    /// 未知20件の一致を帯で出す。設定も台帳も変えない
    JevEval {
        #[arg(long)]
        live: bool,
    },
}

#[derive(Subcommand)]
enum IntakeCmd {
    /// Outlook デスクトップ（COM）からメール・予定を読む
    Outlook {
        /// 例: 3d （既定は config の lookback_days）
        #[arg(long)]
        since: Option<String>,
        /// 保存せず抽出結果を表示
        #[arg(long)]
        dry_run: bool,
    },
    /// フィクスチャ JSON を同じパイプラインに流す
    Fixture {
        dir: std::path::PathBuf,
    },
    /// 決定 / TODO / アクション の行を会議候補にする
    Note {
        file: std::path::PathBuf,
    },
    /// チケットのフィクスチャを候補にする
    Tickets {
        file: std::path::PathBuf,
    },
    /// Teams のフィクスチャを候補にする
    Teams {
        file: std::path::PathBuf,
    },
    /// 自分に割り当てられた GitHub の Issue と PR を候補にする
    Github,
    /// 自分に割り当てられた Jira を候補にする
    Jira,
    /// 直近の Teams チャットを候補にする
    Chat,
    /// 自分に割り当てられた Azure DevOps Boards の作業項目を候補にする
    Devops,
    /// 届いている入口をまとめて読む
    Sync,
    /// ブラウザの Outlook か Teams を、開く・次へ・終わりだけで読む
    Browse {
        /// mail または teams
        #[arg(long, default_value = "mail")]
        site: String,
    },
}

#[derive(Subcommand)]
enum StartupCmd {
    /// HKCU Run（失敗時はスタートアップフォルダ）に登録
    Install,
    /// 登録を削除
    Remove,
    /// どちらで登録されているかを表示
    Status,
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// 既定の config.toml を生成
    Init,
}

#[derive(Subcommand)]
enum CandCmd {
    /// 採用/却下待ちの候補を表示
    List,
    /// 候補を追加
    Add {
        title: String,
        /// 出所（teams / outlook / meeting / alert / manual）
        #[arg(long, default_value = "manual")]
        source: String,
        /// 元メッセージ等への参照。同じ参照は二度と候補にならない
        #[arg(long = "ref")]
        source_ref: Option<String>,
    },
    /// 候補をタスクにする（既定は今日の予定）
    Accept {
        id: String,
        #[arg(long)]
        backlog: bool,
        #[arg(long)]
        date: Option<String>,
    },
    /// 候補を却下
    Reject { id: String },
}

fn finish(outcome: Outcome) -> i32 {
    match outcome {
        Outcome::Done => 0,
        Outcome::Pending(_) => 2,
    }
}

fn run() -> Result<i32> {
    let cli = Cli::parse();
    if let Cmd::Doctor = cli.cmd {
        doctor::run();
        return Ok(0);
    }
    if let Cmd::Where = cli.cmd {
        println!("{}", paths::data_dir().display());
        return Ok(0);
    }
    if let Cmd::Mcp = cli.cmd {
        mcp::run()?;
        return Ok(0);
    }
    if let Cmd::Serve { quiet } = cli.cmd {
        serve::run(quiet)?;
        return Ok(0);
    }
    if let Cmd::Startup(s) = cli.cmd {
        match s {
            StartupCmd::Install => startup::install()?,
            StartupCmd::Remove => startup::remove()?,
            StartupCmd::Status => startup::status()?,
        }
        return Ok(0);
    }
    if let Cmd::Config(c) = cli.cmd {
        match c {
            ConfigCmd::Init => config::init()?,
        }
        return Ok(0);
    }
    let store = Store::open()?;
    let ui = Ui::new(cli.yes);

    let code = match cli.cmd {
        Cmd::Today { date } => {
            let d = resolve_date(date.as_deref())?;
            rituals::print_day(&store, &d)?;
            0
        }
        Cmd::Next { date } => {
            let date = resolve_date(date.as_deref())?;
            match order::prepare_next(&store, &date)? {
                Some(t) => {
                    println!("{}  {}", t.id, t.title);
                    0
                }
                None => {
                    println!("{date} に未完了はありません");
                    0
                }
            }
        }
        Cmd::Prefer { first, second } => {
            graph::record_order(&store, &first, &second)?;
            println!("覚えました: 「{first}」を「{second}」より先");
            0
        }
        Cmd::Add { title, due, estimate, date, backlog } => {
            if let Some(d) = &due {
                parse_date(d)?;
            }
            let plan = if backlog { None } else { Some(resolve_date(date.as_deref())?) };
            let t = store.add_task(&title, due.as_deref(), estimate, "manual", None, plan.as_deref())?;
            println!("追加: {}  {}  [{}]", short(&t.id), t.title, t.state.label_ja());
            if let Some(d) = plan {
                markdown::export(&store, &d)?;
            }
            0
        }
        Cmd::Plan { date } => {
            let d = resolve_date(date.as_deref())?;
            let o = rituals::plan_ritual(&store, &ui, &d)?;
            markdown::export(&store, &d)?;
            finish(o)
        }
        Cmd::Check { date } => {
            let d = resolve_date(date.as_deref())?;
            let o = rituals::check_ritual(&store, &ui, &d)?;
            markdown::export(&store, &d)?;
            finish(o)
        }
        Cmd::Start { id, date } => {
            let mut t = store.get_task(&id)?;
            if t.plan_date.is_none() {
                let d = resolve_date(date.as_deref())?;
                t = store.schedule(&t.id, &d)?;
            }
            let t = store.transition(&t.id, State::InProgress, None, None)?;
            println!("着手: {}  {}", short(&t.id), t.title);
            export_for(&store, &t)?;
            0
        }
        Cmd::Done { id, evidence } => {
            let t = store.transition(&id, State::Done, None, evidence.as_deref())?;
            println!("完了: {}  {}", short(&t.id), t.title);
            export_for(&store, &t)?;
            0
        }
        Cmd::Notdone { id, reason } => {
            let t = store.transition(&id, State::NotDone, Some(&reason), None)?;
            println!("未完了: {}  {}  理由: {reason}", short(&t.id), t.title);
            export_for(&store, &t)?;
            0
        }
        Cmd::Carry { id, reason, to, reschedule } => {
            let old = store.get_task(&id)?;
            let base = old.plan_date.clone().unwrap_or_else(util::today);
            let to = match to {
                Some(t) => {
                    parse_date(&t)?;
                    t
                }
                None => next_workday(&base)?,
            };
            if let Some(r) = &reschedule {
                parse_date(r)?;
            }
            let t = store.carry_over(&old.id, &reason, &to, reschedule.as_deref())?;
            println!("持ち越し: {}  {}  -> {to}（{}回目）", short(&t.id), t.title, t.carried_count);
            export_for(&store, &old)?;
            export_for(&store, &t)?;
            0
        }
        Cmd::Drop { id, reason } => {
            let t = store.transition(&id, State::Dropped, Some(&reason), None)?;
            println!("取り下げ: {}  {}  理由: {reason}", short(&t.id), t.title);
            export_for(&store, &t)?;
            0
        }
        Cmd::Revise { id, choice, reason } => match choice.as_str() {
            "shelve" | "today" | "backlog" | "reject" => {
                let c = store.revise_candidate(&id, &choice)?;
                println!("枝を更新: {}  {}  -> {choice}", short(&c.id), c.title);
                0
            }
            "done" | "not_done" | "carry" | "drop" | "start" => {
                let t = store.revise_task(&id, &choice, reason.as_deref())?;
                println!(
                    "枝を更新: {}  {}  [{}]",
                    short(&t.id),
                    t.title,
                    t.state.label_ja()
                );
                export_for(&store, &t)?;
                0
            }
            other => {
                anyhow::bail!("選択肢が不正です: {other}");
            }
        },
        Cmd::Split { id, reason, into, to } => {
            let old = store.get_task(&id)?;
            let base = old.plan_date.clone().unwrap_or_else(util::today);
            let to = match to {
                Some(t) => {
                    parse_date(&t)?;
                    t
                }
                None => next_workday(&base)?,
            };
            let news = store.split(&old.id, &into, &reason, &to)?;
            println!("分割: {} -> {} 件（{to}）", old.title, news.len());
            for n in &news {
                println!("  {}  {}", short(&n.id), n.title);
            }
            export_for(&store, &old)?;
            markdown::export(&store, &to)?;
            0
        }
        Cmd::Close { date } => {
            let d = resolve_date(date.as_deref())?;
            let o = rituals::close_ritual(&store, &ui, &d)?;
            markdown::export(&store, &d)?;
            finish(o)
        }
        Cmd::Retro { date } => {
            let d = resolve_date(date.as_deref())?;
            finish(rituals::retro_ritual(&store, &ui, &d)?)
        }
        Cmd::Candidates(c) => match c {
            CandCmd::List => {
                let cands = store.open_candidates()?;
                if cands.is_empty() {
                    println!("候補なし");
                }
                for c in cands {
                    let age = util::days_since(&c.created_at);
                    let stale = if age >= store::CANDIDATE_STALE_DAYS { "  !! 放置" } else { "" };
                    println!("{}  {}  ({}、{age}日前){stale}", short(&c.id), c.title, c.source);
                }
                0
            }
            CandCmd::Add { title, source, source_ref } => {
                match store.add_candidate(&title, &source, source_ref.as_deref())? {
                    Some(c) => println!("候補追加: {}  {}", short(&c.id), c.title),
                    None => println!("同じ参照が既に存在するか却下済みのため追加しません"),
                }
                0
            }
            CandCmd::Accept { id, backlog, date } => {
                let plan = if backlog { None } else { Some(resolve_date(date.as_deref())?) };
                let t = store.accept_candidate(&id, plan.as_deref())?;
                println!("採用: {}  {}  [{}]", short(&t.id), t.title, t.state.label_ja());
                export_for(&store, &t)?;
                0
            }
            CandCmd::Reject { id } => {
                store.reject_candidate(&id)?;
                println!("却下しました");
                0
            }
        },
        Cmd::Export { date } => {
            let d = resolve_date(date.as_deref())?;
            let p = markdown::export(&store, &d)?;
            println!("{}", p.display());
            0
        }
        Cmd::Import { date } => {
            let d = resolve_date(date.as_deref())?;
            let r = markdown::import(&store, &d)?;
            println!("取り込み: 完了 {} 件、新規 {} 件", r.completed, r.added);
            markdown::export(&store, &d)?;
            0
        }
        Cmd::Intake(sub) => match sub {
            IntakeCmd::Outlook { since, dry_run } => {
                let cfg = config::load();
                let r = dayloop::intake::run_outlook(&store, &cfg, since.as_deref(), dry_run);
                println!("{}", r.summary_line());
                0
            }
            IntakeCmd::Fixture { dir } => {
                let cfg = config::load();
                let r = dayloop::intake::run_fixture(&store, &dir, &cfg)?;
                println!("{}", r.summary_line());
                0
            }
            IntakeCmd::Note { file } => {
                let text = std::fs::read_to_string(&file)?;
                let r = dayloop::intake::minutes::ingest(&store, &text)?;
                println!(
                    "議事録: 候補 {} 件、共有 {} 件、無視 {} 件",
                    r.actions, r.info, r.ignored
                );
                0
            }
            IntakeCmd::Tickets { file } => {
                let text = std::fs::read_to_string(&file)?;
                let n = dayloop::intake::tickets::ingest(&store, &text)?;
                println!("チケット候補: {n} 件");
                0
            }
            IntakeCmd::Teams { file } => {
                let text = std::fs::read_to_string(&file)?;
                let n = dayloop::intake::teams::ingest(&store, &text)?;
                println!("Teams 候補: {n} 件");
                0
            }
            IntakeCmd::Github => match dayloop::github::fetch_assigned() {
                Ok(items) => {
                    let n = dayloop::intake::github_intake::ingest(&store, &items)?;
                    println!("GitHub 候補: {n} 件");
                    0
                }
                Err(_) => {
                    println!("GitHub に聞けません。GITHUB_TOKEN を置くと Issue と PR を候補にします");
                    0
                }
            },
            IntakeCmd::Jira => match dayloop::jira::fetch_assigned() {
                Ok(items) => {
                    let n = dayloop::intake::jira_live::ingest(&store, &items)?;
                    println!("Jira 候補: {n} 件");
                    0
                }
                Err(_) => {
                    println!("Jira に聞けません。JIRA_BASE_URL、JIRA_EMAIL、JIRA_API_TOKEN を置くとチケットを候補にします");
                    0
                }
            },
            IntakeCmd::Devops => match dayloop::devops::fetch_assigned() {
                Ok(items) => {
                    let n = dayloop::intake::devops_live::ingest(&store, &items)?;
                    println!("DevOps 候補: {n} 件");
                    0
                }
                Err(_) => {
                    println!("Azure DevOps に聞けません。AZURE_DEVOPS_ORG と、PAT または az login が必要です");
                    0
                }
            },
            IntakeCmd::Browse { site } => {
                let n = dayloop::browse::run(&store, &site)?;
                println!("ブラウザから候補: {n} 件");
                0
            }
            IntakeCmd::Sync => {
                let results = dayloop::intake::link(&store);
                if results.iter().all(|r| r.ok && r.candidates_added == 0 && r.events == 0) {
                    println!("新しい候補はありません");
                }
                0
            }
            IntakeCmd::Chat => match dayloop::chat::fetch_recent() {
                Ok(items) => {
                    let n = dayloop::intake::chat_live::ingest(&store, &items)?;
                    println!("Teams 候補: {n} 件");
                    0
                }
                Err(_) => {
                    println!("Teams に聞けません。TEAMS_TOKEN か GRAPH_TOKEN を置くとチャットを候補にします");
                    0
                }
            },
        },
        Cmd::JevEval { live } => run_jev_eval(live)?,
        Cmd::Doctor
        | Cmd::Where
        | Cmd::Mcp
        | Cmd::Serve { .. }
        | Cmd::Startup(_)
        | Cmd::Config(_) => unreachable!(),
    };
    Ok(code)
}

fn run_jev_eval(live: bool) -> Result<i32> {
    let text = std::fs::read_to_string("fixtures/jev-band-20.json")?;
    let mut rows = jev_eval::load_sheet(&text)?;
    if live {
        let cfg = config::load();
        if !jev::ready(&cfg.jev.mode, &cfg.jev.route) {
            eprintln!("Jev のキーがありません。Codex と同じ TYPESAFE_API_KEY を置くか、DAYLOOP_JEV_API_KEY を置いてください");
            return Ok(2);
        }
        let mut decider = jev::HttpDecider {
            route: jev::route_of(&cfg.jev.route),
            timeout_ms: cfg.jev.timeout_ms,
        };
        let choices = vec!["task".to_string(), "info".to_string(), "ask".to_string()];
        for row in &mut rows {
            match decider.decide(&format!("subject={}", row.subject), &choices) {
                jev::JevOutcome::Answer { choice, confidence }
                    if choices.iter().any(|c| c == &choice) =>
                {
                    row.choice = choice;
                    row.confidence = confidence;
                }
                _ => {
                    row.choice.clear();
                    row.confidence = 0.0;
                }
            }
        }
    }
    print!("{}", jev_eval::report(&rows));
    Ok(0)
}

fn export_for(store: &Store, t: &model::Task) -> Result<()> {
    if let Some(d) = &t.plan_date {
        markdown::export(store, d)?;
    }
    Ok(())
}

#[cfg(windows)]
fn set_utf8_console() {
    // Legacy conhost defaults to the OEM code page (932 on Japanese Windows).
    // Switching this process's console to UTF-8 needs no admin rights.
    use windows_sys::Win32::System::Console::{SetConsoleCP, SetConsoleOutputCP};
    unsafe {
        SetConsoleOutputCP(65001);
        SetConsoleCP(65001);
    }
}

#[cfg(not(windows))]
fn set_utf8_console() {}

fn main() {
    set_utf8_console();
    match run() {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("エラー: {e}");
            std::process::exit(1);
        }
    }
}
