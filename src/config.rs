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
"#;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub schedule: Schedule,
    #[serde(default)]
    pub notify: Notify,
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
