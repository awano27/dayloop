//! Environment diagnosis for a locked-down corporate Windows PC (no admin rights).
//! Everything here is read-only.

use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::paths;
use crate::screen;

fn ok(label: &str, detail: &str) {
    println!("[OK] {label}  {detail}");
}
fn ng(label: &str, detail: &str, fix: &str) {
    println!("[NG] {label}  {detail}");
    if !fix.is_empty() {
        println!("     -> {fix}");
    }
}
fn info(label: &str, detail: &str) {
    println!("[--] {label}  {detail}");
}

fn cmd_ok(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(windows)]
fn reg_exists(key: &str) -> bool {
    cmd_ok("reg", &["query", key])
}

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var(var).ok().map(PathBuf::from)
}

fn first_existing(paths: &[PathBuf]) -> Option<PathBuf> {
    paths.iter().find(|p| p.exists()).cloned()
}

pub fn run() {
    println!("dayloop doctor");
    println!();

    // Data directory
    let dir = paths::data_dir();
    let probe = dir.join(".probe");
    let writable = std::fs::create_dir_all(&dir)
        .and_then(|_| std::fs::write(&probe, b"ok"))
        .is_ok();
    let _ = std::fs::remove_file(&probe);
    if writable {
        ok("データ領域", &dir.display().to_string());
    } else {
        ng(
            "データ領域",
            &dir.display().to_string(),
            "書き込みできません。環境変数 DAYLOOP_HOME で別の場所を指定してください",
        );
    }

    // This binary is running, which already answers "can user-area exe run".
    match std::env::current_exe() {
        Ok(p) => ok("exe の実行", &p.display().to_string()),
        Err(_) => info("exe の実行", "パス不明"),
    }
    match screen::locate_wincli() {
        Some(p) => ok("画面の読み取り", &p.display().to_string()),
        None => info(
            "画面の読み取り",
            "wincli か Sbroenne.WindowsMcp.exe が無いと dayloop capture は取得失敗になります",
        ),
    }

    let mut prefer_vscode = false;

    #[cfg(windows)]
    {
        let local = env_path("LOCALAPPDATA").unwrap_or_default();
        let pf = env_path("ProgramFiles").unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));
        let pf86 = env_path("ProgramFiles(x86)").unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)"));

        // AppLocker / WDAC policy presence (informational: we are running, so this exe is allowed now)
        let applocker = reg_exists(r"HKLM\SOFTWARE\Policies\Microsoft\Windows\SrpV2");
        let wdac = Path::new(r"C:\Windows\System32\CodeIntegrity\CiPolicies\Active").exists()
            || Path::new(r"C:\Windows\System32\CodeIntegrity\SIPolicy.p7b").exists();
        if applocker || wdac {
            info(
                "アプリ制御ポリシー",
                &format!(
                    "検出（AppLocker: {}, WDAC: {}）。この exe は起動できていますが、更新後に止まる場合は VS Code 拡張版へ",
                    applocker, wdac
                ),
            );
            prefer_vscode = true;
        } else {
            ok("アプリ制御ポリシー", "設定なし");
        }

        // Outlook desktop COM: actually create Outlook.Application and read Version.
        let new_outlook = local.join(r"Microsoft\WindowsApps\olk.exe").exists();
        match crate::intake::outlook_com::probe_version() {
            Ok(ver) => ok("Outlook COM", &format!("バージョン {ver}")),
            Err(e) => ng(
                "Outlook COM",
                &e.to_string(),
                "メール・予定は段階4の Edge 経由になります",
            ),
        }
        if new_outlook {
            info("新しい Outlook", "検出。COM が使えない場合は Edge 経由に切り替えます");
        }
        if crate::config::load().intake.read_body {
            info(
                "intake.read_body",
                "true: 本文を読むため、Outlook のオブジェクトモデルガードが出る可能性があります",
            );
        }

        // Edge
        let edge = first_existing(&[
            pf86.join(r"Microsoft\Edge\Application\msedge.exe"),
            pf.join(r"Microsoft\Edge\Application\msedge.exe"),
        ]);
        match edge {
            Some(p) => ok("Microsoft Edge", &p.display().to_string()),
            None => ng("Microsoft Edge", "見つかりません", "勤怠・Teams のブラウザ経由取得には Edge が必要です"),
        }

        // Task Scheduler as the current user
        if cmd_ok("schtasks", &["/Query", "/FO", "LIST"]) {
            ok("タスクスケジューラ (ユーザー権限)", "定時実行に使えます");
        } else {
            ng(
                "タスクスケジューラ (ユーザー権限)",
                "利用できません",
                "スタートアップ常駐（HKCU Run）に切り替えます",
            );
        }

        // HKCU Run key (user-writable unless blocked by policy)
        if reg_exists(r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run") {
            ok("HKCU Run キー", "ログオン時常駐に使えます");
        } else {
            ng("HKCU Run キー", "読めません", "スタートアップフォルダのショートカットを使います");
        }
        let startup = env_path("APPDATA")
            .unwrap_or_default()
            .join(r"Microsoft\Windows\Start Menu\Programs\Startup");
        if startup.exists() {
            ok("スタートアップフォルダ", &startup.display().to_string());
        }

        // VS Code
        let code = first_existing(&[
            local.join(r"Programs\Microsoft VS Code\Code.exe"),
            pf.join(r"Microsoft VS Code\Code.exe"),
        ]);
        match code {
            Some(p) => ok("VS Code", &p.display().to_string()),
            None => {
                info("VS Code", "見つかりません（exe 禁止 PC では VS Code 拡張版が必要）");
                if prefer_vscode {
                    println!("     !! exe 制御ポリシーがあり VS Code も無いため、この PC は対象外の可能性があります");
                }
            }
        }

        // Local LLM runtimes
        let lm = local.join(r"Programs\LM Studio\LM Studio.exe").exists()
            || local.join(r"LM-Studio\LM Studio.exe").exists();
        let ollama = local.join(r"Programs\Ollama\ollama.exe").exists();
        match (lm, ollama) {
            (true, true) => ok("ローカル LLM", "LM Studio と Ollama を検出"),
            (true, false) => ok("ローカル LLM", "LM Studio を検出"),
            (false, true) => ok("ローカル LLM", "Ollama を検出"),
            (false, false) => info("ローカル LLM", "未検出（無くても動きます）"),
        }

        // Proxy
        match std::env::var("HTTPS_PROXY").or_else(|_| std::env::var("HTTP_PROXY")) {
            Ok(p) => info("プロキシ", &p),
            Err(_) => info("プロキシ", "環境変数なし（OS 設定を継承します）"),
        }
    }

    #[cfg(not(windows))]
    {
        info("Windows 固有チェック", "この OS では省略");
    }

    // LLM endpoint reachability
    match std::env::var("DAYLOOP_LLM_ENDPOINT") {
        Ok(ep) => {
            let hostport = ep
                .trim_start_matches("http://")
                .trim_start_matches("https://")
                .split('/')
                .next()
                .unwrap_or("")
                .to_string();
            let hostport = if hostport.contains(':') {
                hostport
            } else if ep.starts_with("https") {
                format!("{hostport}:443")
            } else {
                format!("{hostport}:80")
            };
            let reachable = hostport
                .to_socket_addrs()
                .ok()
                .and_then(|mut a| a.next())
                .map(|a| TcpStream::connect_timeout(&a, Duration::from_secs(2)).is_ok())
                .unwrap_or(false);
            if reachable {
                ok("LLM エンドポイント", &ep);
            } else {
                ng("LLM エンドポイント", &ep, "接続できません。テンプレート出力で動作します");
            }
        }
        Err(_) => info("LLM エンドポイント", "未設定（テンプレート出力で動作します）"),
    }

    if !crate::jev::api_key().is_empty() {
        ok("Jev", "Codex と同じ TypeSafe のキーを使います");
    } else {
        info("Jev", "API キー未設定（未知の節は質問のまま）");
    }

    println!();
    if prefer_vscode {
        println!("推奨: VS Code 拡張版。exe 版は現時点では動いていますが、ポリシー更新で止まる可能性があります。");
    } else {
        println!("推奨: 単一 exe 版。管理者権限なしでそのまま使えます。");
    }
}
