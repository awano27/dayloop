use std::path::PathBuf;

/// Data directory. Never requires admin rights:
/// - DAYLOOP_HOME if set
/// - %LOCALAPPDATA%\dayloop on Windows
/// - ~/.local/share/dayloop elsewhere
pub fn data_dir() -> PathBuf {
    if let Ok(p) = std::env::var("DAYLOOP_HOME") {
        return PathBuf::from(p);
    }
    if let Ok(p) = std::env::var("LOCALAPPDATA") {
        return PathBuf::from(p).join("dayloop");
    }
    if let Ok(h) = std::env::var("HOME") {
        return PathBuf::from(h).join(".local").join("share").join("dayloop");
    }
    PathBuf::from(".").join("dayloop")
}

pub fn db_path() -> PathBuf {
    data_dir().join("dayloop.db")
}

pub fn days_dir() -> PathBuf {
    data_dir().join("days")
}

pub fn day_md_path(date: &str) -> PathBuf {
    days_dir().join(format!("{date}.md"))
}
