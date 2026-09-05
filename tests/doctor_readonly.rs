use std::process::Command;
#[test]
fn doctor_never_creates_a_ledger_or_echoes_endpoint_credentials() {
    let home = std::env::temp_dir().join(format!("dayloop-doctor-{}", ulid::Ulid::new()));
    let out = Command::new(env!("CARGO_BIN_EXE_dayloop"))
        .arg("doctor")
        .env("DAYLOOP_HOME", &home)
        .env("HTTPS_PROXY", "https://user:DO-NOT-ECHO-PROXY@127.0.0.1:1")
        .env(
            "DAYLOOP_LLM_ENDPOINT",
            "https://user:DO-NOT-ECHO-ENDPOINT@127.0.0.1:1/secret",
        )
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(!text.contains("DO-NOT-ECHO"));
    assert!(!home.exists(), "doctor must not initialize data");
    assert!(text.contains("未実装"));
}
