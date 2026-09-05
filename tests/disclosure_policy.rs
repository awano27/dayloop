use dayloop::config::Config;
use dayloop::policy::{AccessPolicy, Profile};
use serde_json::json;

#[test]
fn cloud_profile_requires_its_own_explicit_opt_in() {
    assert!(AccessPolicy::new(Profile::GithubCopilot, &Config::default()).is_err());
    assert!(AccessPolicy::new(Profile::Local, &Config::default()).is_ok());
    let cfg: Config = toml::from_str("[intake]\noutlook=true\nread_body=true").unwrap();
    assert!(AccessPolicy::new(Profile::GithubCopilot, &cfg).is_err());
}

#[test]
fn enabled_cloud_profile_keeps_titles_but_excludes_content_even_in_nested_results() {
    let cfg: Config = toml::from_str("[ai.github_copilot]\nenabled=true").unwrap();
    let policy = AccessPolicy::new(Profile::GithubCopilot, &cfg).unwrap();
    let filtered = policy.filter(json!({
        "tasks":[{"id":"test-id","title":"明日の準備","evidence":"SECRET-EVIDENCE","state_reason":"SECRET-REASON","source_ref":"SECRET-REF"}],
        "observations":[{"title":"件名","body":"SECRET-BODY","excerpt":"SECRET-EXCERPT","future_payload":"SECRET-FUTURE"}],
        "day":{"date":"2026-09-07","retro_note":"SECRET-RETRO"},
        "questions":[{"kind":"close_task","question":"準備を終えましたか","options":[{"label":"完了","tool":"finish_task","args":{"id":"test-id","evidence":"SECRET-ARG"},"needs":["evidence"]}]}]
    }));
    let encoded = filtered.to_string();
    assert!(!encoded.contains("SECRET"), "{encoded}");
    assert_eq!(filtered["tasks"][0]["title"], "明日の準備");
    assert_eq!(
        filtered["questions"][0]["options"][0]["tool"],
        "finish_task"
    );
    assert_eq!(
        filtered["questions"][0]["options"][0]["args"]["id"],
        "test-id"
    );
}

#[test]
fn cloud_content_permissions_are_independent_and_errors_do_not_echo_local_data() {
    let cfg: Config =
        toml::from_str("[ai.github_copilot]\nenabled=true\nallow_bodies=true\nallow_titles=false")
            .unwrap();
    let policy = AccessPolicy::new(Profile::GithubCopilot, &cfg).unwrap();
    let v = policy.filter(json!({"title":"SECRET-TITLE","question":"SECRET-TITLEを終えましたか","body":"許可した本文","evidence":"SECRET-EVIDENCE","retro_note":"SECRET-NOTE","error":"SECRET-LOCAL-ERROR"}));
    assert!(!v.to_string().contains("SECRET"));
    assert_eq!(v["body"], "許可した本文");
    assert!(v["error"].is_string());
    assert_eq!(
        policy.filter(json!({"error":"carry_blocked"}))["error"],
        "carry_blocked"
    );
}

#[test]
fn local_profile_preserves_the_original_payload() {
    let value = json!({"body":"本文","future_field":{"note":"メモ"},"error":"詳細"});
    assert_eq!(
        AccessPolicy::new(Profile::Local, &Config::default())
            .unwrap()
            .filter(value.clone()),
        value
    );
}

#[test]
fn invalid_configuration_fails_without_echoing_its_source_text() {
    let dir = std::env::temp_dir().join(format!("dayloop-policy-{}", ulid::Ulid::new()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    std::fs::write(&path, "[ai.github_copilot]\nenabled='SECRET-CONFIG'").unwrap();
    let err = dayloop::config::load_at(&path).unwrap_err().to_string();
    assert!(!err.contains("SECRET-CONFIG"));
    assert!(err.contains("config.toml"));
    std::fs::remove_dir_all(dir).unwrap();
}
