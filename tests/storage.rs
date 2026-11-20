//! Settings, status file and data directory selection.

use lamzu_battery::config::{Paths, is_writable_dir, pick_data_dir};
use lamzu_battery::i18n::Language;
use lamzu_battery::settings::{SettingsStore, StatusPayload, StatusStore, UpdateMode, unix_now};
use serde_json::{Value, json};

fn store(directory: &std::path::Path) -> SettingsStore {
    SettingsStore::new(directory.join("settings.json"))
}

#[test]
fn settings_are_merged_not_replaced() {
    let directory = tempfile::tempdir().expect("temp dir");
    let settings = store(directory.path());
    settings
        .set("language", Value::String("ru".to_owned()))
        .expect("write");
    settings
        .set("update_mode", Value::String("notify".to_owned()))
        .expect("write");
    let values = settings.values();
    assert_eq!(values.get("language").and_then(Value::as_str), Some("ru"));
    assert_eq!(
        values.get("update_mode").and_then(Value::as_str),
        Some("notify")
    );
    assert_eq!(settings.language(), Some(Language::Ru));
    assert_eq!(settings.update_mode(), UpdateMode::Notify);
}

#[test]
fn unknown_update_mode_falls_back_to_auto() {
    let directory = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        directory.path().join("settings.json"),
        json!({"language": "en", "update_mode": "sometimes"}).to_string(),
    )
    .expect("write");
    let settings = store(directory.path());
    assert_eq!(settings.update_mode(), UpdateMode::Auto);
    assert_eq!(settings.language(), Some(Language::En));
}

#[test]
fn unknown_keys_are_preserved() {
    let directory = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        directory.path().join("settings.json"),
        json!({"future_key": "keep me"}).to_string(),
    )
    .expect("write");
    let settings = store(directory.path());
    settings
        .set("language", Value::String("en".to_owned()))
        .expect("write");
    let values = settings.values();
    assert_eq!(
        values.get("future_key").and_then(Value::as_str),
        Some("keep me")
    );
    assert_eq!(values.get("language").and_then(Value::as_str), Some("en"));
}

#[test]
fn settings_with_bom_are_read() {
    let directory = tempfile::tempdir().expect("temp dir");
    let mut bytes = b"\xEF\xBB\xBF".to_vec();
    bytes.extend_from_slice(
        json!({"language": "ru", "update_mode": "notify"})
            .to_string()
            .as_bytes(),
    );
    std::fs::write(directory.path().join("settings.json"), bytes).expect("write");
    let settings = store(directory.path());
    assert_eq!(settings.language(), Some(Language::Ru));
    assert_eq!(settings.update_mode(), UpdateMode::Notify);
}

#[test]
fn no_temporary_files_are_left_behind() {
    let directory = tempfile::tempdir().expect("temp dir");
    store(directory.path())
        .set("language", Value::String("ru".to_owned()))
        .expect("write");
    let leftovers: Vec<_> = std::fs::read_dir(directory.path())
        .expect("read")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "tmp"))
        .collect();
    assert!(leftovers.is_empty());
}

#[test]
fn missing_settings_file_is_tolerated() {
    let directory = tempfile::tempdir().expect("temp dir");
    let settings = store(directory.path());
    assert!(settings.values().is_empty());
    assert_eq!(settings.language(), None);
    assert_eq!(settings.update_mode(), UpdateMode::Auto);
}

#[test]
fn writable_probe_leaves_no_files() {
    let directory = tempfile::tempdir().expect("temp dir");
    assert!(is_writable_dir(directory.path()));
    assert_eq!(
        std::fs::read_dir(directory.path()).expect("read").count(),
        0
    );
}

#[test]
fn writable_probe_reports_failure() {
    let directory = tempfile::tempdir().expect("temp dir");
    let blocker = directory.path().join("file");
    std::fs::write(&blocker, b"x").expect("write");
    assert!(!is_writable_dir(&blocker.join("data")));
}

#[test]
fn data_directory_falls_back_when_the_app_folder_is_read_only() {
    let directory = tempfile::tempdir().expect("temp dir");
    let base = directory.path().join("app");
    let local = directory.path().join("local");
    std::fs::create_dir_all(&base).expect("create");
    std::fs::create_dir_all(&local).expect("create");
    let preferred = base.join("data");
    std::fs::create_dir_all(&preferred).expect("create");
    // Make the portable directory unusable by replacing it with a file.
    std::fs::remove_dir_all(&preferred).expect("remove");
    std::fs::write(&preferred, b"blocked").expect("write");
    let (chosen, fallback) = pick_data_dir(&base, Some(&local), directory.path());
    assert_eq!(chosen, local.join(lamzu_battery::APP_ID));
    assert_eq!(fallback, Some(preferred));
}

#[test]
fn data_directory_prefers_the_folder_next_to_the_application() {
    let directory = tempfile::tempdir().expect("temp dir");
    let base = directory.path().join("app");
    std::fs::create_dir_all(&base).expect("create");
    let (chosen, fallback) = pick_data_dir(&base, None, directory.path());
    assert_eq!(chosen, base.join("data"));
    assert_eq!(fallback, None);
}

#[test]
fn status_payload_contains_the_updater_contract() {
    let directory = tempfile::tempdir().expect("temp dir");
    let status = StatusStore::new(directory.path().join("status.json"));
    let payload = StatusPayload {
        app: lamzu_battery::APP_NAME.to_owned(),
        version: lamzu_battery::VERSION.to_owned(),
        pid: 42,
        display_name: "LAMZU Thorn".to_owned(),
        language: "ru".to_owned(),
        model: Some("Thorn".to_owned()),
        protocol: Some("compx".to_owned()),
        product: Some("LAMZU THORN".to_owned()),
        running: true,
        healthy: true,
        heartbeat_unix: unix_now(),
        online: true,
        percent: Some(42),
        state: Some(1),
        raw_percent: Some(40),
        voltage_mv: Some(4000),
        charging: true,
        full: false,
        updated_unix: Some(unix_now()),
        polled_unix: unix_now(),
        update_last_check_unix: None,
        update_last_error: None,
    };
    status.write(&payload).expect("write");
    let values = status.values();
    for key in [
        "app",
        "version",
        "pid",
        "display_name",
        "language",
        "model",
        "protocol",
        "product",
        "running",
        "healthy",
        "heartbeat_unix",
        "online",
        "percent",
        "state",
        "charging",
        "full",
        "updated_unix",
        "polled_unix",
    ] {
        assert!(values.contains_key(key), "missing {key}");
    }
    assert_eq!(values.get("percent").and_then(Value::as_u64), Some(42));
    assert_eq!(values.get("charging").and_then(Value::as_bool), Some(true));
}

#[test]
fn health_snapshot_is_read_with_bom_and_tolerates_missing_fields() {
    let directory = tempfile::tempdir().expect("temp dir");
    let status = StatusStore::new(directory.path().join("status.json"));
    let mut bytes = b"\xEF\xBB\xBF".to_vec();
    bytes.extend_from_slice(
        json!({"version": "1.6.0", "healthy": true, "heartbeat_unix": 10})
            .to_string()
            .as_bytes(),
    );
    std::fs::write(directory.path().join("status.json"), bytes).expect("write");
    let health = status.health().expect("health");
    assert_eq!(health.version, "1.6.0");
    assert!(health.healthy);
    assert_eq!(health.heartbeat_unix, 10);

    std::fs::write(directory.path().join("status.json"), b"{}").expect("write");
    let health = status.health().expect("health");
    assert!(!health.healthy);
    assert_eq!(health.heartbeat_unix, 0);
}

#[test]
fn paths_use_the_same_file_names_as_the_python_version() {
    let directory = tempfile::tempdir().expect("temp dir");
    let paths = Paths::for_base(
        directory.path().to_path_buf(),
        directory.path().join("LamzuBattery.exe"),
    );
    assert!(paths.status.ends_with("status.json"));
    assert!(paths.settings.ends_with("settings.json"));
    assert!(paths.log.ends_with("app.log"));
    assert!(paths.crash.ends_with("crash.log"));
    assert_eq!(paths.data_dir, directory.path().join("data"));
}
