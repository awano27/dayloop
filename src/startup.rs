use anyhow::Result;

const VALUE_NAME: &str = "dayloop";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

pub fn install() -> Result<()> {
    #[cfg(not(windows))]
    {
        anyhow::bail!("startup は Windows のみです");
    }
    #[cfg(windows)]
    {
        let cmd = serve_cmd()?;
        match write_run(&cmd) {
            Ok(()) => {
                println!("HKCU Run に登録しました: {cmd}");
                Ok(())
            }
            Err(e) => {
                eprintln!("HKCU Run に書けません（{e}）。スタートアップフォルダに落とします");
                write_startup_cmd(&cmd)?;
                println!("スタートアップフォルダに登録しました: {}", startup_cmd_path()?.display());
                Ok(())
            }
        }
    }
}

pub fn remove() -> Result<()> {
    #[cfg(not(windows))]
    {
        anyhow::bail!("startup は Windows のみです");
    }
    #[cfg(windows)]
    {
        let mut any = false;
        match delete_run() {
            Ok(true) => {
                println!("HKCU Run の dayloop を削除しました");
                any = true;
            }
            Ok(false) => {}
            Err(e) => eprintln!("HKCU Run の削除に失敗: {e}"),
        }
        let p = startup_cmd_path()?;
        if p.exists() {
            std::fs::remove_file(&p)?;
            println!("スタートアップフォルダの dayloop.cmd を削除しました");
            any = true;
        }
        if !any {
            println!("登録はありません");
        }
        Ok(())
    }
}

pub fn status() -> Result<()> {
    #[cfg(not(windows))]
    {
        anyhow::bail!("startup は Windows のみです");
    }
    #[cfg(windows)]
    {
        match read_run() {
            Ok(Some(v)) => println!("HKCU Run: {v}"),
            Ok(None) => println!("HKCU Run: （なし）"),
            Err(e) => println!("HKCU Run: 読めません（{e}）"),
        }
        let p = startup_cmd_path()?;
        if p.exists() {
            println!("スタートアップフォルダ: {}", p.display());
        } else {
            println!("スタートアップフォルダ: （なし）");
        }
        Ok(())
    }
}

#[cfg(windows)]
fn serve_cmd() -> Result<String> {
    let exe = std::env::current_exe()?;
    Ok(format!("\"{}\" serve --quiet", exe.display()))
}

#[cfg(windows)]
fn write_run(cmd: &str) -> Result<()> {
    use winreg::enums::*;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(RUN_KEY)?;
    key.set_value(VALUE_NAME, &cmd.to_string())?;
    Ok(())
}

#[cfg(windows)]
fn delete_run() -> Result<bool> {
    use winreg::enums::*;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = match hkcu.open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE | KEY_QUERY_VALUE) {
        Ok(k) => k,
        Err(_) => return Ok(false),
    };
    match key.delete_value(VALUE_NAME) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}

#[cfg(windows)]
fn read_run() -> Result<Option<String>> {
    use winreg::enums::*;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu.open_subkey(RUN_KEY)?;
    match key.get_value::<String, _>(VALUE_NAME) {
        Ok(v) => Ok(Some(v)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

#[cfg(windows)]
fn startup_cmd_path() -> Result<std::path::PathBuf> {
    let appdata = std::env::var("APPDATA").map_err(|_| anyhow::anyhow!("APPDATA がありません"))?;
    Ok(std::path::PathBuf::from(appdata)
        .join(r"Microsoft\Windows\Start Menu\Programs\Startup")
        .join("dayloop.cmd"))
}

#[cfg(windows)]
fn write_startup_cmd(cmd: &str) -> Result<()> {
    let p = startup_cmd_path()?;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&p, format!("{cmd}\r\n"))?;
    Ok(())
}
