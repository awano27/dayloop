use std::io::Write;

use crate::paths;
use crate::util;

pub enum Sent {
    ToastOk,
    ToastFailedFile { err: String },
    FileOk,
    None,
}

impl Sent {
    pub fn log_line(&self) -> String {
        match self {
            Sent::ToastOk => "notify toast ok".into(),
            Sent::ToastFailedFile { err } => {
                let err = err.replace(['\n', '\r'], " ");
                format!("notify toast failed: {err} -> file")
            }
            Sent::FileOk => "notify file ok".into(),
            Sent::None => "notify none".into(),
        }
    }
}

/// Send a user-visible notice. `method` is toast | file | none.
/// Toast failures fall back to file. Non-Windows always uses file for "toast".
pub fn send(method: &str, message: &str) -> Sent {
    match method {
        "none" => Sent::None,
        "file" => {
            write_file(message);
            Sent::FileOk
        }
        _ => {
            #[cfg(windows)]
            {
                match toast(message) {
                    Ok(()) => Sent::ToastOk,
                    Err(e) => {
                        write_file(message);
                        Sent::ToastFailedFile { err: e }
                    }
                }
            }
            #[cfg(not(windows))]
            {
                write_file(message);
                Sent::ToastFailedFile {
                    err: "unsupported platform".into(),
                }
            }
        }
    }
}

fn write_file(message: &str) {
    let p = paths::notify_file_path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let line = format!("{} {}\n", util::now(), message);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = f.write_all(line.as_bytes());
    }
    eprintln!("{message}");
}

#[cfg(windows)]
fn toast(message: &str) -> Result<(), String> {
    // PowerShell's AUMID works without registering an AppID (no admin).
    winrt_notification::Toast::new(winrt_notification::Toast::POWERSHELL_APP_ID)
        .title("dayloop")
        .text1(message)
        .show()
        .map_err(|e| e.to_string())
}
