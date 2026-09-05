use anyhow::Result;
use serde::Deserialize;

use crate::paths;

pub const DEFAULT_TOML: &str = r#"[schedule]
plan  = "08:30"
check = "13:00"
close = "18:00"
retro = "Fri 18:00"
workdays = ["Mon","Tue","Wed","Thu","Fri"]

[notify]
method = "toast"   # toast | file | none

[intake]
outlook = false
lookback_days = 3
read_body = false
keywords = ["お願い", "ご対応", "ご確認", "依頼", "までに", "締切", "期限", "至急", "回答", "deadline", "please", "action required", "ASAP"]
important_senders = []
meeting_prep = true
meeting_prep_only_required = true

# Cloud clients receive the fields enabled here. Text pasted directly into a
# cloud chat has already left the PC and cannot be controlled by dayloop.
[ai.github_copilot]
enabled = false
allow_titles = true
allow_source_refs = false
allow_evidence = false
allow_notes = false
allow_bodies = false
"#;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub schedule: Schedule,
    #[serde(default)]
    pub notify: Notify,
    #[serde(default)]
    pub intake: IntakeConfig,
    #[serde(default)]
    pub ai: AiConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AiConfig {
    pub github_copilot: CloudProfile,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CloudProfile {
    pub enabled: bool,
    pub allow_titles: bool,
    pub allow_source_refs: bool,
    pub allow_evidence: bool,
    pub allow_notes: bool,
    pub allow_bodies: bool,
}

impl Default for CloudProfile {
    fn default() -> Self {
        Self {
            enabled: false,
            allow_titles: true,
            allow_source_refs: false,
            allow_evidence: false,
            allow_notes: false,
            allow_bodies: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
    "Fri 18:00".into()
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
    false
}
fn default_lookback() -> i64 {
    3
}
fn default_true() -> bool {
    true
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
    match load_at(&paths::config_path()) {
        Ok(config) => config,
        Err(_) => {
            eprintln!("config.toml が読めません。外部連携を無効にした既定値を使います");
            Config::default()
        }
    }
}

pub fn load_at(path: &std::path::Path) -> Result<Config> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(_) => anyhow::bail!("config.toml を読み取れません。ファイルの権限を確認してください"),
    };
    let config: Config = toml::from_str(&text).map_err(|_| {
        anyhow::anyhow!("config.toml の形式が不正です。設定項目と値の型を確認してください")
    })?;
    validate(&config)?;
    Ok(config)
}

fn validate(config: &Config) -> Result<()> {
    let hm = |s: &str| chrono::NaiveTime::parse_from_str(s, "%H:%M").ok();
    let schedule = &config.schedule;
    let times = [hm(&schedule.plan), hm(&schedule.check), hm(&schedule.close)];
    let valid_days = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    let valid_day = |day: &str| valid_days.iter().any(|v| day.eq_ignore_ascii_case(v));
    let retro = schedule
        .retro
        .split_once(' ')
        .map(|(day, time)| valid_day(day) && hm(time).is_some())
        .unwrap_or_else(|| hm(&schedule.retro).is_some());
    if times.iter().any(Option::is_none)
        || !(times[0] < times[1] && times[1] < times[2])
        || !retro
        || schedule.workdays.iter().any(|d| !valid_day(d))
    {
        anyhow::bail!(
            "config.toml の曜日・時刻が不正です。plan < check < close の順で指定してください"
        );
    }
    if !["toast", "file", "none"].contains(&config.notify.method.as_str())
        || !(0..=365).contains(&config.intake.lookback_days)
    {
        anyhow::bail!("config.toml の通知方式または取得日数が不正です");
    }
    Ok(())
}

pub fn init() -> Result<()> {
    std::fs::create_dir_all(paths::data_dir())?;
    let p = paths::config_path();
    if p.exists() {
        println!("既にあります: {}", p.display());
        return Ok(());
    }
    use std::io::Write;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&p)?
        .write_all(DEFAULT_TOML.as_bytes())?;
    println!("{}", p.display());
    Ok(())
}
