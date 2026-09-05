//! Explicit HKCU startup registration. No scripts, fallback folders or unrelated deletion.
use anyhow::Result;

#[cfg(windows)]
const VALUE_NAME: &str = "dayloop";
#[cfg(windows)]
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

pub fn install() -> Result<()> {
    #[cfg(not(windows))]
    anyhow::bail!("startup は Windows のみです");
    #[cfg(windows)]
    {
        use winreg::{enums::*, RegKey};
        let command = serve_cmd()?;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu.create_subkey(RUN_KEY)?;
        let current = value(&key)?;
        if !may_install(current.as_deref(), &command) {
            anyhow::bail!("HKCU Run の dayloop は別の登録です。上書きせず終了します");
        }
        key.set_value(VALUE_NAME, &command)?;
        println!("HKCU Run に登録しました。この実行ファイルとデータ領域で serve を起動します。");
        Ok(())
    }
}

pub fn remove() -> Result<()> {
    #[cfg(not(windows))]
    anyhow::bail!("startup は Windows のみです");
    #[cfg(windows)]
    {
        use winreg::{enums::*, RegKey};
        let expected = serve_cmd()?;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let key = match hkcu.open_subkey_with_flags(RUN_KEY, KEY_QUERY_VALUE | KEY_SET_VALUE) {
            Ok(key) => key,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                println!("登録はありません");
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        };
        let current = value(&key)?;
        match current {
            None => println!("登録はありません"),
            Some(current) if current == expected => {
                key.delete_value(VALUE_NAME)?;
                println!("この実行ファイルの HKCU Run 登録を削除しました");
            }
            Some(_) => anyhow::bail!(
                "HKCU Run の dayloop は別の実行ファイル・データ領域の登録です。削除しません"
            ),
        }
        Ok(())
    }
}

pub fn status() -> Result<()> {
    #[cfg(not(windows))]
    anyhow::bail!("startup は Windows のみです");
    #[cfg(windows)]
    {
        use winreg::{enums::*, RegKey};
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let current = match hkcu.open_subkey(RUN_KEY) {
            Ok(key) => value(&key)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        match current {
            None => println!("HKCU Run: 登録なし"),
            Some(command) if command == serve_cmd()? => {
                println!("HKCU Run: この実行ファイル・データ領域で登録済み")
            }
            Some(_) => println!("HKCU Run: 別の登録あり（自動変更しません）"),
        }
        Ok(())
    }
}

#[cfg(any(windows, test))]
fn may_install(current: Option<&str>, expected: &str) -> bool {
    current.is_none_or(|value| value == expected)
}

#[cfg(windows)]
fn serve_cmd() -> Result<String> {
    let exe = std::env::current_exe()?;
    let data = crate::paths::data_dir();
    if !data.is_absolute() {
        anyhow::bail!("startup には絶対パスの DAYLOOP_HOME が必要です");
    }
    Ok(format!(
        "\"{}\" serve --quiet --data-dir \"{}\"",
        exe.display(),
        data.display()
    ))
}

#[cfg(windows)]
fn value(key: &winreg::RegKey) -> Result<Option<String>> {
    match key.get_value::<String, _>(VALUE_NAME) {
        Ok(value) => Ok(Some(value)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn install_preserves_different_existing_registration() {
        let own = "\"C:\\apps\\dayloop.exe\" serve --quiet --data-dir \"C:\\data\\dayloop\"";
        assert!(may_install(None, own));
        assert!(may_install(Some(own), own));
        assert!(!may_install(Some("another program"), own));
        assert!(!may_install(
            Some("\"C:\\apps\\dayloop.exe\" serve --quiet"),
            own
        ));
    }
}
