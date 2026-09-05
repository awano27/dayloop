//! Read-only capability and ledger diagnosis. Never creates data or launches services.
use crate::{config, paths, store::Store};

pub fn run() {
    println!("dayloop doctor");
    let dir = paths::data_dir();
    println!(
        "[--] データ領域: {}（書き込みテストはしません）",
        dir.display()
    );
    println!("[OK] CLI / Markdown / MCP stdio: 実装済み");
    println!("[OK] 7カテゴリ確認 / 定期タスク / 明示ACTIONメモ候補: 実装済み");
    println!("[--] Edge自動取得 / exe不要VS Code拡張 / Graph・M365: 未実装");
    match config::load_at(&paths::config_path()) {
        Ok(cfg) => {
            println!(
                "[--] Outlook接続許可: {} / 本文取得許可: {}",
                cfg.intake.outlook, cfg.intake.read_body
            );
            println!(
                "[--] GitHub Copilot出力許可: {} / 本文開示許可: {}",
                cfg.ai.github_copilot.enabled, cfg.ai.github_copilot.allow_bodies
            );
        }
        Err(_) => println!("[NG] config.toml: 読み取りまたは形式エラー（内容は表示しません）"),
    }
    if paths::db_path().exists() {
        match Store::open_read_only(paths::db_path()).and_then(|s| s.integrity_issues()) {
            Ok(issues) if issues.is_empty() => println!("[OK] 台帳診断: 検出された矛盾なし"),
            Ok(issues) => println!(
                "[NG] 台帳診断: {} 件。dayloop ledger check で確認してください",
                issues.len()
            ),
            Err(_) => println!("[NG] 台帳診断: 読み取れないか非対応形式。変更・移行は行いません"),
        }
    } else {
        println!("[--] 台帳: 未作成。dayloop setup で初期化できます");
    }
    println!(
        "[--] プロキシ環境変数: {}（値は表示しません）",
        if std::env::var_os("HTTPS_PROXY")
            .or_else(|| std::env::var_os("HTTP_PROXY"))
            .is_some()
        {
            "設定あり"
        } else {
            "設定なし"
        }
    );
    println!("[--] LLM endpoint環境変数: {}（値・接続は確認しません。dayloop自体の推論接続設定ではありません）",if std::env::var_os("DAYLOOP_LLM_ENDPOINT").is_some() {"設定あり"} else {"設定なし"});
    #[cfg(windows)]
    windows_capabilities();
    #[cfg(not(windows))]
    println!("[--] Windows通知・HKCU startup・Outlook COM: このOSでは利用不可");
    println!("[--] インストール検出は認証・接続成功・企業ポリシーの許可を証明しません");
}

#[cfg(windows)]
fn windows_capabilities() {
    use std::path::PathBuf;
    use winreg::{enums::*, RegKey};
    let com = RegKey::predef(HKEY_CLASSES_ROOT)
        .open_subkey("Outlook.Application\\CLSID")
        .is_ok();
    println!(
        "[--] Outlook COM登録: {}（起動・認証はしません）",
        if com { "検出" } else { "未検出" }
    );
    let local = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_default();
    let pf = std::env::var_os("ProgramFiles")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));
    let pf86 = std::env::var_os("ProgramFiles(x86)")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)"));
    for (label, candidates) in [
        (
            "Edge",
            vec![
                pf.join(r"Microsoft\Edge\Application\msedge.exe"),
                pf86.join(r"Microsoft\Edge\Application\msedge.exe"),
            ],
        ),
        (
            "VS Code",
            vec![
                local.join(r"Programs\Microsoft VS Code\Code.exe"),
                pf.join(r"Microsoft VS Code\Code.exe"),
            ],
        ),
        (
            "LM Studio",
            vec![
                local.join(r"Programs\LM Studio\LM Studio.exe"),
                local.join(r"LM-Studio\LM Studio.exe"),
            ],
        ),
        ("Ollama", vec![local.join(r"Programs\Ollama\ollama.exe")]),
    ] {
        println!(
            "[--] {label}: {}",
            if candidates.iter().any(|p| p.is_file()) {
                "標準インストール先で検出"
            } else {
                "標準インストール先では未検出"
            }
        );
    }
    println!("[--] Windows通知: Win32 balloon + ファイルfallback（表示・既読は未確認）");
    println!("[--] 自動起動: 明示的な dayloop startup install 時のみHKCU Runへ登録");
}
