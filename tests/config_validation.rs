#[test]
fn invalid_schedule_or_unknown_optin_is_rejected_without_echoing_values() {
    let dir = std::env::temp_dir().join(format!("dayloop-config-validation-{}", ulid::Ulid::new()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    for text in [
        "[schedule]\nplan='99:00'",
        "[schedule]\nplan='19:00'",
        "[schedule]\nretro='xxx 18:00'",
        "[notify]\nmethod='secret-unknown'",
        "[intake]\noutlok=true",
        "[ai.github_copilot]\nenable=true",
        "[intake]\nlookback_days=9223372036854775807",
    ] {
        std::fs::write(&path, text).unwrap();
        let error = dayloop::config::load_at(&path).unwrap_err().to_string();
        assert!(!error.contains("secret-unknown"));
    }
    std::fs::write(&path, dayloop::config::DEFAULT_TOML).unwrap();
    assert!(dayloop::config::load_at(&path).is_ok());
    std::fs::remove_dir_all(dir).unwrap();
}
