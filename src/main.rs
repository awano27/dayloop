use anyhow::Result;
use clap::{Parser, Subcommand};

use dayloop::config;
use dayloop::doctor;
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
    /// 初期設定を生成（既存設定と接続許可は変更しない）
    Setup,
    /// 7カテゴリの日次確認
    #[command(subcommand)]
    Reviews(ReviewCmd),
    /// 曜日ごとの定期タスク
    #[command(subcommand)]
    Routines(RoutineCmd),
    /// ACTION/TODO/宿題/対応 行を候補として取り込む
    Note {
        #[arg(long)]
        category: String,
        #[arg(long)]
        title: String,
        #[arg(long, conflicts_with = "file")]
        body: Option<String>,
        #[arg(long, conflicts_with = "body")]
        file: Option<std::path::PathBuf>,
        #[arg(long)]
        source_ref: Option<String>,
        #[arg(long)]
        meeting_id: Option<String>,
    },
    /// 閉鎖済みの日を理由付きで再開（タスクの結果は変更しない）
    Reopen {
        #[arg(long)]
        date: Option<String>,
        #[arg(long)]
        reason: String,
    },
    /// 台帳の整合性を診断
    #[command(subcommand)]
    Ledger(LedgerCmd),
    /// SQLiteの整合したバックアップを作る（既存ファイルは上書きしない）
    Backup {
        #[arg(long)]
        output: Option<std::path::PathBuf>,
    },
    /// 今日の状態を表示
    Today {
        #[arg(long)]
        date: Option<String>,
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
        /// 本人の振り返りを保存する
        #[arg(long)]
        note: Option<String>,
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
    Doctor {
        /// 検証通知を送る（省略時は読み取り専用。失敗時はデータ領域のファイルに保存）
        #[arg(long)]
        notify_test: bool,
    },
    /// データの場所を表示
    Where,
    /// MCP サーバー（stdio）。stdout は JSON-RPC 専用
    Mcp {
        /// local | github-copilot（クラウド送信はconfig.tomlで個別に許可）
        #[arg(long, value_enum, default_value_t = dayloop::policy::Profile::Local)]
        profile: dayloop::policy::Profile,
    },
    /// 常駐し、設定した時刻に非対話で plan / check / close / retro を実行する
    Serve {
        /// コンソールウィンドウを出さない（ログオン時向け）
        #[arg(long)]
        quiet: bool,
        /// 常駐時のデータ領域（startup登録で引き継ぐ絶対パス）
        #[arg(long)]
        data_dir: Option<std::path::PathBuf>,
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
}

#[derive(Subcommand)]
enum LedgerCmd {
    Check {
        #[arg(long)]
        json: bool,
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
    Fixture { dir: std::path::PathBuf },
}

#[derive(Subcommand)]
enum StartupCmd {
    /// HKCU Run に明示登録（別の登録は上書きしない）
    Install,
    /// 登録を削除
    Remove,
    /// HKCU Run 登録状態を表示
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

#[derive(Subcommand)]
enum ReviewCmd {
    List {
        #[arg(long)]
        date: Option<String>,
    },
    Record {
        category: String,
        outcome: String,
        #[arg(long)]
        date: Option<String>,
        #[arg(long)]
        reason: Option<String>,
        #[arg(long)]
        task_id: Option<String>,
        #[arg(long)]
        candidate_id: Option<String>,
    },
    /// 次に初めて準備する日から適用。--none はタスクだけの運用
    Configure {
        #[arg(
            long,
            value_delimiter = ',',
            required_unless_present = "none",
            conflicts_with = "none"
        )]
        categories: Vec<String>,
        #[arg(long)]
        none: bool,
    },
}

#[derive(Subcommand)]
enum RoutineCmd {
    List,
    Add {
        title: String,
        #[arg(long, value_delimiter = ',', required = true)]
        weekdays: Vec<String>,
        #[arg(long)]
        starts_on: String,
    },
    Enable {
        id: String,
    },
    Disable {
        id: String,
    },
}

fn print_tool(store: &Store, name: &str, args: serde_json::Value) -> Result<i32> {
    let result = dayloop::tools::dispatch(store, name, &args);
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(if result.get("error").is_some() { 1 } else { 0 })
}

fn finish(outcome: Outcome) -> i32 {
    match outcome {
        Outcome::Done => 0,
        Outcome::Pending(_) => 2,
    }
}

fn run() -> Result<i32> {
    let cli = Cli::parse();
    if let Cmd::Doctor { notify_test } = cli.cmd {
        doctor::run();
        if notify_test {
            println!(
                "{}",
                dayloop::notify::send(
                    "toast",
                    "dayloop の通知テストです。業務の状態は変更していません。"
                )?
                .log_line()
            );
        }
        return Ok(0);
    }
    if let Cmd::Where = cli.cmd {
        println!("{}", paths::data_dir().display());
        return Ok(0);
    }
    if let Cmd::Mcp { profile } = cli.cmd {
        mcp::run_with_profile(profile)?;
        return Ok(0);
    }
    if let Cmd::Serve { quiet, data_dir } = cli.cmd {
        if let Some(dir) = data_dir {
            if !dir.is_absolute() {
                anyhow::bail!("--data-dir は絶対パスで指定してください");
            }
            std::env::set_var("DAYLOOP_HOME", dir);
        }
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
    let store = if matches!(cli.cmd, Cmd::Ledger(_)) {
        Store::open_read_only(paths::db_path())?
    } else {
        Store::open()?
    };
    let ui = Ui::new(cli.yes);

    let code = match cli.cmd {
        Cmd::Setup => {
            config::init()?;
            println!("データ: {}", store.data_dir().display());
            println!(
                "必須確認: {}",
                store
                    .required_categories()?
                    .iter()
                    .map(|c| c.label_ja())
                    .collect::<Vec<_>>()
                    .join("、")
            );
            println!("接続は個別に設定します。次に dayloop plan を実行してください。");
            0
        }
        Cmd::Reviews(sub) => {
            let (name, args) = match sub {
                ReviewCmd::List { date } => (
                    "list_reviews",
                    serde_json::json!({"date":resolve_date(date.as_deref())?}),
                ),
                ReviewCmd::Record {
                    category,
                    outcome,
                    date,
                    reason,
                    task_id,
                    candidate_id,
                } => {
                    let mut a = serde_json::json!({"date":resolve_date(date.as_deref())?,"category":category,"outcome":outcome});
                    for (key, value) in [
                        ("reason", reason),
                        ("task_id", task_id),
                        ("candidate_id", candidate_id),
                    ] {
                        if let Some(v) = value {
                            a[key] = serde_json::json!(v);
                        }
                    }
                    ("record_review", a)
                }
                ReviewCmd::Configure {
                    categories,
                    none: _,
                } => (
                    "configure_reviews",
                    serde_json::json!({"categories":categories}),
                ),
            };
            print_tool(&store, name, args)?
        }
        Cmd::Routines(sub) => {
            let (name, args) = match sub {
                RoutineCmd::List => ("list_routines", serde_json::json!({})),
                RoutineCmd::Add {
                    title,
                    weekdays,
                    starts_on,
                } => (
                    "add_routine",
                    serde_json::json!({"title":title,"weekdays":weekdays,"starts_on":starts_on}),
                ),
                RoutineCmd::Enable { id } => (
                    "set_routine_enabled",
                    serde_json::json!({"id":id,"enabled":true}),
                ),
                RoutineCmd::Disable { id } => (
                    "set_routine_enabled",
                    serde_json::json!({"id":id,"enabled":false}),
                ),
            };
            print_tool(&store, name, args)?
        }
        Cmd::Note {
            category,
            title,
            body,
            file,
            source_ref,
            meeting_id,
        } => {
            let body = match (body, file) {
                (Some(s), _) => s,
                (_, Some(p)) => std::fs::read_to_string(p)?,
                _ => anyhow::bail!("--body または --file が必要です"),
            };
            let mut args = serde_json::json!({"category":category,"title":title,"body":body});
            if let Some(v) = source_ref {
                args["source_ref"] = serde_json::json!(v);
            }
            if let Some(v) = meeting_id {
                args["meeting_id"] = serde_json::json!(v);
            }
            print_tool(&store, "ingest_note", args)?
        }
        Cmd::Reopen { date, reason } => {
            let d = resolve_date(date.as_deref())?;
            store.reopen_day(&d, &reason)?;
            warn_mirror(&store, &d);
            println!("{d} を再開しました。タスクの結果は保持されています。");
            0
        }
        Cmd::Ledger(LedgerCmd::Check { json }) => {
            let issues = store.integrity_issues()?;
            let ok = issues.is_empty();
            if json {
                println!("{}", serde_json::json!({"ok":ok,"issues":issues}));
            } else if ok {
                println!("台帳の不整合は見つかりませんでした");
            } else {
                for issue in &issues {
                    println!(
                        "{} {} {}",
                        issue.code,
                        issue.date.as_deref().unwrap_or("未計画"),
                        issue.detail
                    );
                }
            }
            if ok {
                0
            } else {
                1
            }
        }
        Cmd::Backup { output } => {
            let output = output.unwrap_or_else(|| {
                store
                    .data_dir()
                    .join("backups")
                    .join(format!("dayloop-{}.db", ulid::Ulid::new()))
            });
            let output = if output.is_absolute() {
                output
            } else {
                store.data_dir().join(output)
            };
            if output
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                anyhow::bail!("バックアップ先はdayloopのデータ領域内を指定してください");
            }
            let ancestor = output
                .ancestors()
                .find(|p| p.exists())
                .ok_or_else(|| anyhow::anyhow!("保存先を確認できません"))?;
            if !std::fs::canonicalize(ancestor)?.starts_with(store.data_dir()) {
                anyhow::bail!("保存先がデータ領域の外を参照しています");
            }
            let output = std::fs::canonicalize(ancestor)?.join(output.strip_prefix(ancestor)?);
            store.backup_to(&output)?;
            println!("{}", output.display());
            0
        }
        Cmd::Today { date } => {
            let d = resolve_date(date.as_deref())?;
            store.prepare_day(&d)?;
            rituals::print_day(&store, &d)?;
            0
        }
        Cmd::Add {
            title,
            due,
            estimate,
            date,
            backlog,
        } => {
            if let Some(d) = &due {
                parse_date(d)?;
            }
            let plan = if backlog {
                None
            } else {
                Some(resolve_date(date.as_deref())?)
            };
            let t = store.add_task(
                &title,
                due.as_deref(),
                estimate,
                "manual",
                None,
                plan.as_deref(),
            )?;
            println!(
                "追加: {}  {}  [{}]",
                short(&t.id),
                t.title,
                t.state.label_ja()
            );
            if let Some(d) = plan {
                warn_mirror(&store, &d);
            }
            0
        }
        Cmd::Plan { date } => {
            let d = resolve_date(date.as_deref())?;
            let o = rituals::plan_ritual(&store, &ui, &d)?;
            warn_mirror(&store, &d);
            finish(o)
        }
        Cmd::Check { date } => {
            let d = resolve_date(date.as_deref())?;
            let o = rituals::check_ritual(&store, &ui, &d)?;
            warn_mirror(&store, &d);
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
        Cmd::Carry {
            id,
            reason,
            to,
            reschedule,
        } => {
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
            println!(
                "持ち越し: {}  {}  -> {to}（{}回目）",
                short(&t.id),
                t.title,
                t.carried_count
            );
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
        Cmd::Split {
            id,
            reason,
            into,
            to,
        } => {
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
            warn_mirror(&store, &to);
            0
        }
        Cmd::Close { date } => {
            let d = resolve_date(date.as_deref())?;
            let o = rituals::close_ritual(&store, &ui, &d)?;
            warn_mirror(&store, &d);
            finish(o)
        }
        Cmd::Retro { date, note } => {
            let d = resolve_date(date.as_deref())?;
            if let Some(note) = note {
                return print_tool(
                    &store,
                    "save_retro",
                    serde_json::json!({"date":d,"note":note}),
                );
            }
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
                    let stale = if age >= store::CANDIDATE_STALE_DAYS {
                        "  !! 放置"
                    } else {
                        ""
                    };
                    println!(
                        "{}  {}  ({}、{age}日前){stale}",
                        short(&c.id),
                        c.title,
                        c.source
                    );
                }
                0
            }
            CandCmd::Add {
                title,
                source,
                source_ref,
            } => {
                match store.add_candidate(&title, &source, source_ref.as_deref())? {
                    Some(c) => println!("候補追加: {}  {}", short(&c.id), c.title),
                    None => println!("同じ参照が既に存在するか却下済みのため追加しません"),
                }
                0
            }
            CandCmd::Accept { id, backlog, date } => {
                let plan = if backlog {
                    None
                } else {
                    Some(resolve_date(date.as_deref())?)
                };
                let t = store.accept_candidate(&id, plan.as_deref())?;
                println!(
                    "採用: {}  {}  [{}]",
                    short(&t.id),
                    t.title,
                    t.state.label_ja()
                );
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
            warn_mirror(&store, &d);
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
        },
        Cmd::Doctor { .. }
        | Cmd::Where
        | Cmd::Mcp { .. }
        | Cmd::Serve { .. }
        | Cmd::Startup(_)
        | Cmd::Config(_) => unreachable!(),
    };
    Ok(code)
}

fn warn_mirror(store: &Store, date: &str) {
    if markdown::export(store, date).is_err() {
        eprintln!("注意: 台帳への操作は反映されていますが、{date} のMarkdown出力に失敗しました。権限・ファイルのロックを確認し、exportを再実行してください。");
    }
}

fn export_for(store: &Store, t: &model::Task) -> Result<()> {
    if let Some(d) = &t.plan_date {
        warn_mirror(store, d);
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
