use anyhow::Result;
use serde::Deserialize;

use crate::paths;

pub const DEFAULT_TOML: &str = r#"[schedule]
plan  = "08:30"
check = "13:00"
close = "18:00"
retro = "Fri 18:30"
workdays = ["Mon","Tue","Wed","Thu","Fri"]

[notify]
method = "toast"   # toast | file | none

[intake]
outlook = true
lookback_days = 3
read_body = false
keywords = ["お願い", "ご対応", "ご確認", "依頼", "までに", "締切", "期限", "至急", "回答", "deadline", "please", "action required", "ASAP"]
important_senders = []
meeting_prep = true
meeting_prep_only_required = true

[jev]
mode = "off"
route = ""
timeout_ms = 2000
send_body = false
"#;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub schedule: Schedule,
    #[serde(default)]
    pub notify: Notify,
    #[serde(default)]
    pub intake: IntakeConfig,
    #[serde(default)]
    pub jev: JevConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IntakeConfig {
    #[serde(default = "default_outlook")]
    pub outlook: bool,
    #[serde(default = "default_lookback")]
    pub lookback_days: i64,
    #[serde(default)]
    pub read_body: bool,
    #[serde(default = "default_keywords")]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub important_senders: Vec<String>,
    #[serde(default = "default_true")]
    pub meeting_prep: bool,
    #[serde(default = "default_true")]
    pub meeting_prep_only_required: bool,
}

impl Default for IntakeConfig {
    fn default() -> Self {
        Self {
            outlook: default_outlook(),
            lookback_days: default_lookback(),
            read_body: false,
            keywords: default_keywords(),
            important_senders: Vec::new(),
            meeting_prep: true,
            meeting_prep_only_required: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Schedule {
    #[serde(default = "default_plan")]
    pub plan: String,
    #[serde(default = "default_check")]
    pub check: String,
    #[serde(default = "default_close")]
    pub close: String,
    #[serde(default = "default_retro")]
    pub retro: String,
    #[serde(default = "default_workdays")]
    pub workdays: Vec<String>,
}

impl Default for Schedule {
    fn default() -> Self {
        Self {
            plan: default_plan(),
            check: default_check(),
            close: default_close(),
            retro: default_retro(),
            workdays: default_workdays(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Notify {
    #[serde(default = "default_method")]
    pub method: String,
}

impl Default for Notify {
    fn default() -> Self {
        Self {
            method: default_method(),
        }
    }
}

fn default_plan() -> String {
    "08:30".into()
}
fn default_check() -> String {
    "13:00".into()
}
fn default_close() -> String {
    "18:00".into()
}
fn default_retro() -> String {
    "Fri 18:30".into()
}
fn default_workdays() -> Vec<String> {
    ["Mon", "Tue", "Wed", "Thu", "Fri"]
        .into_iter()
        .map(|s| s.to_string())
        .collect()
}
fn default_method() -> String {
    "toast".into()
}
fn default_outlook() -> bool {
    true
}
fn default_lookback() -> i64 {
    3
}
fn default_true() -> bool {
    true
}
#[derive(Debug, Clone, Deserialize)]
pub struct JevConfig {
    #[serde(default = "default_jev_mode")]
    pub mode: String,
    #[serde(default)]
    pub route: String,
    #[serde(default = "default_jev_timeout")]
    pub timeout_ms: u64,
    /// Set only after the eval sheet in docs/jev-eval.md. Absent means no auto-commit.
    #[serde(default)]
    pub commit_confidence: Option<f64>,
    #[serde(default)]
    pub send_body: bool,
}

impl Default for JevConfig {
    fn default() -> Self {
        Self {
            mode: default_jev_mode(),
            route: String::new(),
            timeout_ms: default_jev_timeout(),
            commit_confidence: None,
            send_body: false,
        }
    }
}

fn default_jev_mode() -> String {
    "off".into()
}

fn default_jev_timeout() -> u64 {
    2000
}

fn default_keywords() -> Vec<String> {
    [
        "お願い",
        "ご対応",
        "ご確認",
        "依頼",
        "までに",
        "締切",
        "期限",
        "至急",
        "回答",
        "deadline",
        "please",
        "action required",
        "ASAP",
    ]
    .into_iter()
    .map(|s| s.to_string())
    .collect()
}

pub fn load() -> Config {
    let p = paths::config_path();
    match std::fs::read_to_string(&p) {
        Ok(s) => toml::from_str(&s).unwrap_or_else(|e| {
            eprintln!("config.toml が読めないため既定値を使います: {e}");
            Config::default()
        }),
        Err(_) => Config::default(),
    }
}

pub fn init() -> Result<()> {
    std::fs::create_dir_all(paths::data_dir())?;
    let p = paths::config_path();
    if p.exists() {
        println!("既にあります: {}", p.display());
        return Ok(());
    }
    std::fs::write(&p, DEFAULT_TOML)?;
    println!("{}", p.display());
    Ok(())
}
