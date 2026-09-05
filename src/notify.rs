use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};

use crate::paths;
use crate::util;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Sent {
    NativeAccepted,
    NativeFailedFile { err: String },
    FileOk,
    Disabled,
}

impl Sent {
    pub fn log_line(&self) -> String {
        match self {
            Self::NativeAccepted => "notify native accepted".into(),
            Self::NativeFailedFile { err } => {
                format!(
                    "notify native failed: {} -> file",
                    err.replace(['\n', '\r'], " ")
                )
            }
            Self::FileOk => "notify file ok".into(),
            Self::Disabled => "notify disabled".into(),
        }
    }

    pub fn is_delivery(&self) -> bool {
        matches!(
            self,
            Self::NativeAccepted | Self::NativeFailedFile { .. } | Self::FileOk
        )
    }
}

/// Send a notice with the configured method. `none` is distinct from a
/// notification failure, and all true file-write failures are returned.
pub fn send(method: &str, message: &str) -> Result<Sent> {
    send_to_path(method, message, &paths::notify_file_path())
}

pub fn send_to_path(method: &str, message: &str, file_path: &Path) -> Result<Sent> {
    match method.trim().to_ascii_lowercase().as_str() {
        "none" => Ok(Sent::Disabled),
        "file" => {
            write_file(file_path, message)?;
            Ok(Sent::FileOk)
        }
        _ => native_or_file(message, file_path),
    }
}

fn native_or_file(message: &str, file_path: &Path) -> Result<Sent> {
    #[cfg(windows)]
    let native = native_balloon(message);
    #[cfg(not(windows))]
    let native: Result<()> = Err(anyhow::anyhow!("unsupported platform"));

    match native {
        Ok(()) => Ok(Sent::NativeAccepted),
        Err(error) => {
            let error_text = error.to_string();
            write_file(file_path, message).with_context(|| {
                format!("native notification failed ({error_text}); file fallback also failed")
            })?;
            Ok(Sent::NativeFailedFile { err: error_text })
        }
    }
}

fn write_file(path: &Path, message: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("通知フォルダを作成できません: {}", dir.display()))?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("通知ファイルを開けません: {}", path.display()))?;
    file.write_all(format!("{} {}\n", util::now(), message).as_bytes())
        .with_context(|| format!("通知ファイルへ書き込めません: {}", path.display()))?;
    file.sync_data()
        .with_context(|| format!("通知ファイルを同期できません: {}", path.display()))?;
    Ok(())
}

#[cfg(windows)]
fn native_balloon(message: &str) -> Result<()> {
    use std::mem::size_of;
    use std::thread;
    use std::time::Duration;

    use windows_sys::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY,
        NOTIFYICONDATAW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, LoadIconW, IDI_INFORMATION, WS_POPUP,
    };

    fn copy_wide(target: &mut [u16], value: &str) {
        let writable = target.len().saturating_sub(1);
        for (slot, unit) in target.iter_mut().take(writable).zip(value.encode_utf16()) {
            *slot = unit;
        }
    }

    // Use an owned hidden window: `serve --quiet` may have detached from a
    // console, while a parent terminal must remain visible and untouched.
    let static_class: Vec<u16> = "STATIC\0".encode_utf16().collect();
    let title: Vec<u16> = "dayloop\0".encode_utf16().collect();
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            static_class.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    if hwnd.is_null() {
        anyhow::bail!("owned notification window could not be created");
    }
    let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = 0x444C;
    data.uFlags = NIF_TIP | NIF_ICON;
    data.hIcon = unsafe { LoadIconW(std::ptr::null_mut(), IDI_INFORMATION) };
    if data.hIcon.is_null() {
        unsafe { DestroyWindow(hwnd) };
        anyhow::bail!("default notification icon could not be loaded");
    }
    copy_wide(&mut data.szTip, "dayloop");
    if unsafe { Shell_NotifyIconW(NIM_ADD, &data) } == 0 {
        unsafe { DestroyWindow(hwnd) };
        anyhow::bail!("Shell_NotifyIcon NIM_ADD was rejected");
    }
    data.uFlags = NIF_INFO;
    data.dwInfoFlags = NIIF_INFO;
    copy_wide(&mut data.szInfoTitle, "dayloop");
    copy_wide(&mut data.szInfo, message);
    let shown = unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) } != 0;
    // This confirms system acceptance only; it cannot prove a person read it.
    thread::sleep(Duration::from_secs(5));
    unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
    unsafe { DestroyWindow(hwnd) };
    if !shown {
        anyhow::bail!("Shell_NotifyIcon NIM_MODIFY was rejected");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_errors_are_propagated() {
        let dir = std::env::temp_dir().join(format!("dayloop-notify-{}", ulid::Ulid::new()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(send_to_path("file", "test", &dir).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn disabled_is_not_a_delivery() {
        assert_eq!(
            send_to_path("none", "test", Path::new("unused")).unwrap(),
            Sent::Disabled
        );
    }
}
