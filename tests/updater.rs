//! Version handling, update paths, preflight checks, replacement and rollback.

use std::time::{Duration, SystemTime};

use lamzu_battery::settings::HealthSnapshot;
use lamzu_battery::updater::{
    self, UpdateStage, Version, ensure_update_target, health_ok, is_newer, normalize_digest,
    replace_executable, rollback_executable, same_version, update_paths,
};

#[test]
fn versions_are_parsed_and_compared_numerically() {
    assert_eq!(
        Version::parse("v1.2.1").map(|v| v.to_string()),
        Some("1.2.1".to_owned())
    );
    assert_eq!(
        Version::parse("LAMZU Battery 1.10.0").map(|v| v.to_string()),
        Some("1.10.0".to_owned())
    );
    assert!(Version::parse("no version").is_none());
    assert!(is_newer("v1.2.2", "1.2.1"));
    assert!(is_newer("v1.10.0", "1.9.9"));
    assert!(is_newer("v2.0", "1.99.99"));
    assert!(!is_newer("v1.2.1", "1.2.1"));
    assert!(!is_newer("garbage", "1.2.1"));
    assert!(same_version("v1.6", "1.6.0"));
    assert!(same_version("1.6.0.0", "1.6"));
    assert!(same_version("1.5.10", "v1.5.10"));
    assert!(!same_version("1.6.1", "1.6.0"));
    assert!(!same_version("", "1.6.0"));
}

#[test]
fn update_paths_use_the_actual_file_name() {
    let directory = tempfile::tempdir().expect("temp dir");
    let target = directory.path().join("Custom Name.exe");
    let (new_path, backup) = update_paths(&target);
    assert_eq!(new_path, directory.path().join("Custom Name.new.exe"));
    assert_eq!(backup, directory.path().join("Custom Name.old.exe"));
}

#[test]
fn digests_are_normalized() {
    let digest = "a".repeat(64);
    assert_eq!(
        normalize_digest(Some(&format!("sha256:{digest}"))),
        Some(digest.clone())
    );
    assert_eq!(normalize_digest(Some(&digest.to_uppercase())), Some(digest));
    assert_eq!(normalize_digest(Some("nope")), None);
    assert_eq!(normalize_digest(None), None);
}

#[test]
fn preflight_accepts_a_writable_folder() {
    let directory = tempfile::tempdir().expect("temp dir");
    let target = directory.path().join("LamzuBattery.exe");
    std::fs::write(&target, b"current").expect("write");
    ensure_update_target(&target, 1024).expect("preflight should pass");
}

#[test]
fn preflight_reports_a_missing_executable() {
    let directory = tempfile::tempdir().expect("temp dir");
    let error =
        ensure_update_target(&directory.path().join("missing.exe"), 0).expect_err("must fail");
    assert_eq!(error.stage(), UpdateStage::Preflight);
    assert!(error.message().contains("not found"));
}

#[test]
#[expect(
    clippy::permissions_set_readonly_false,
    reason = "the temporary file must become writable again so the directory can be removed"
)]
fn preflight_reports_a_read_only_executable() {
    let directory = tempfile::tempdir().expect("temp dir");
    let target = directory.path().join("LamzuBattery.exe");
    std::fs::write(&target, b"current").expect("write");
    let mut permissions = std::fs::metadata(&target).expect("metadata").permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&target, permissions).expect("readonly");
    let error = ensure_update_target(&target, 0).expect_err("must fail");
    assert!(error.message().contains("read-only"), "{}", error.message());
    let mut permissions = std::fs::metadata(&target).expect("metadata").permissions();
    permissions.set_readonly(false);
    std::fs::set_permissions(&target, permissions).expect("writable");
}

#[test]
fn preflight_reports_missing_space() {
    let directory = tempfile::tempdir().expect("temp dir");
    let target = directory.path().join("LamzuBattery.exe");
    std::fs::write(&target, b"current").expect("write");
    let error = ensure_update_target(&target, 1 << 50).expect_err("must fail");
    assert!(
        error.message().contains("free space"),
        "{}",
        error.message()
    );
}

#[test]
fn replacement_keeps_the_rollback_copy() {
    let directory = tempfile::tempdir().expect("temp dir");
    let target = directory.path().join("LamzuBattery.exe");
    let new_path = directory.path().join("LamzuBattery.new.exe");
    std::fs::write(&target, b"old").expect("write");
    std::fs::write(&new_path, b"new").expect("write");
    let backup = replace_executable(&target, &new_path, None).expect("replace");
    assert_eq!(std::fs::read(&target).expect("read"), b"new");
    assert_eq!(std::fs::read(&backup).expect("read"), b"old");
    assert!(!new_path.exists());
}

#[test]
fn replacement_refreshes_the_backup_timestamp() {
    let directory = tempfile::tempdir().expect("temp dir");
    let target = directory.path().join("LamzuBattery.exe");
    let new_path = directory.path().join("LamzuBattery.new.exe");
    std::fs::write(&target, b"old").expect("write");
    std::fs::write(&new_path, b"new").expect("write");
    let old = SystemTime::now() - Duration::from_secs(7 * 24 * 60 * 60);
    let file = std::fs::File::options()
        .write(true)
        .open(&target)
        .expect("open");
    file.set_modified(old).expect("set mtime");
    let backup = replace_executable(&target, &new_path, None).expect("replace");
    let age = SystemTime::now()
        .duration_since(
            std::fs::metadata(&backup)
                .expect("metadata")
                .modified()
                .expect("mtime"),
        )
        .expect("age");
    assert!(age < Duration::from_secs(60), "backup looks stale: {age:?}");
}

#[test]
fn replacement_refuses_a_fresh_backup() {
    let directory = tempfile::tempdir().expect("temp dir");
    let target = directory.path().join("LamzuBattery.exe");
    let new_path = directory.path().join("LamzuBattery.new.exe");
    let backup = directory.path().join("LamzuBattery.old.exe");
    std::fs::write(&target, b"current").expect("write");
    std::fs::write(&new_path, b"new").expect("write");
    std::fs::write(&backup, b"previous").expect("write");
    let error = replace_executable(&target, &new_path, Some(&backup)).expect_err("must refuse");
    assert_eq!(error.stage(), UpdateStage::Install);
    assert_eq!(std::fs::read(&target).expect("read"), b"current");
    assert_eq!(std::fs::read(&backup).expect("read"), b"previous");
}

#[test]
fn rollback_swaps_the_files_back() {
    let directory = tempfile::tempdir().expect("temp dir");
    let target = directory.path().join("LamzuBattery.exe");
    let (new_path, backup) = update_paths(&target);
    std::fs::write(&target, b"new").expect("write");
    std::fs::write(&backup, b"old").expect("write");
    assert!(rollback_executable(&target, &backup, &new_path));
    assert_eq!(std::fs::read(&target).expect("read"), b"old");
    assert_eq!(std::fs::read(&new_path).expect("read"), b"new");
    assert!(!backup.exists());
}

#[test]
fn rollback_restores_a_missing_executable() {
    let directory = tempfile::tempdir().expect("temp dir");
    let target = directory.path().join("LamzuBattery.exe");
    let (new_path, backup) = update_paths(&target);
    std::fs::write(&backup, b"old").expect("write");
    std::fs::write(&new_path, b"new").expect("write");
    assert!(rollback_executable(&target, &backup, &new_path));
    assert_eq!(std::fs::read(&target).expect("read"), b"old");
    assert!(!backup.exists());
}

#[test]
fn rollback_without_a_backup_is_a_noop() {
    let directory = tempfile::tempdir().expect("temp dir");
    let target = directory.path().join("LamzuBattery.exe");
    let (new_path, backup) = update_paths(&target);
    std::fs::write(&target, b"new").expect("write");
    assert!(!rollback_executable(&target, &backup, &new_path));
    assert_eq!(std::fs::read(&target).expect("read"), b"new");
}

#[test]
fn download_cleanup_keeps_the_rollback_copy() {
    let directory = tempfile::tempdir().expect("temp dir");
    let target = directory.path().join("LamzuBattery.exe");
    let (new_path, backup) = update_paths(&target);
    std::fs::write(&target, b"current").expect("write");
    std::fs::write(&new_path, b"new").expect("write");
    std::fs::write(&backup, b"old").expect("write");
    std::fs::write(
        new_path.with_file_name("LamzuBattery.new.exe.part"),
        b"partial",
    )
    .expect("write");
    updater::cleanup_download_files(&target);
    assert!(!new_path.exists());
    assert!(
        !new_path
            .with_file_name("LamzuBattery.new.exe.part")
            .exists()
    );
    assert!(backup.exists());
    assert!(target.exists());
}

#[test]
fn backup_cleanup_keeps_the_download() {
    let directory = tempfile::tempdir().expect("temp dir");
    let target = directory.path().join("LamzuBattery.exe");
    let (new_path, backup) = update_paths(&target);
    std::fs::write(&target, b"current").expect("write");
    std::fs::write(&new_path, b"new").expect("write");
    std::fs::write(&backup, b"old").expect("write");
    updater::cleanup_backup(&target);
    assert!(!backup.exists());
    assert!(new_path.exists());
}

#[test]
fn stale_backups_are_removed_but_fresh_ones_are_kept() {
    let directory = tempfile::tempdir().expect("temp dir");
    let target = directory.path().join("LamzuBattery.exe");
    let (_, backup) = update_paths(&target);
    std::fs::write(&target, b"current").expect("write");
    std::fs::write(&backup, b"old").expect("write");
    assert!(!updater::cleanup_stale_backup(&target));
    assert!(backup.exists());

    let old = SystemTime::now() - Duration::from_secs(25 * 60 * 60);
    let file = std::fs::File::options()
        .write(true)
        .open(&backup)
        .expect("open");
    file.set_modified(old).expect("set mtime");
    assert!(updater::cleanup_stale_backup(&target));
    assert!(!backup.exists());
}

#[test]
fn prepare_update_target_removes_a_fresh_backup() {
    let directory = tempfile::tempdir().expect("temp dir");
    let target = directory.path().join("LamzuBattery.exe");
    let (_, backup) = update_paths(&target);
    std::fs::write(&target, b"current").expect("write");
    std::fs::write(&backup, b"old").expect("write");
    assert!(updater::prepare_update_target(
        &target,
        Duration::from_millis(100)
    ));
    assert!(!backup.exists());
}

#[test]
fn health_check_accepts_a_fresh_matching_status() {
    let now = SystemTime::now();
    let heartbeat = lamzu_battery::settings::unix_now();
    let status = HealthSnapshot {
        version: lamzu_battery::VERSION.to_owned(),
        healthy: true,
        heartbeat_unix: heartbeat,
    };
    assert!(health_ok(Some(&status), lamzu_battery::VERSION, now));
    assert!(health_ok(Some(&status), "1.6", now));
    assert!(!health_ok(Some(&status), "9.9.9", now));
    assert!(!health_ok(None, lamzu_battery::VERSION, now));
    let unhealthy = HealthSnapshot {
        healthy: false,
        ..status.clone()
    };
    assert!(!health_ok(Some(&unhealthy), lamzu_battery::VERSION, now));
    let stale = HealthSnapshot {
        heartbeat_unix: heartbeat - 3600,
        ..status
    };
    assert!(!health_ok(Some(&stale), lamzu_battery::VERSION, now));
}

#[test]
fn url_policy_only_trusts_the_official_repository() {
    let policy = lamzu_battery::updater::http::UrlPolicy::github();
    assert!(policy.trusted_asset(
        "https://github.com/lavandiI/lamzu-battery/releases/download/v1.3.0/LamzuBattery.exe"
    ));
    assert!(policy.trusted_asset(
        "https://github.com/Lavandii/Lamzu-Battery/releases/download/v1.3.0/LamzuBattery.exe"
    ));
    assert!(!policy.trusted_asset("https://example.com/LamzuBattery.exe"));
    assert!(!policy.trusted_asset("http://github.com/lavandiI/lamzu-battery/releases/download/x"));
    assert!(
        !policy
            .trusted_asset("https://github.com/other/repo/releases/download/v1/LamzuBattery.exe")
    );
    assert!(policy.trusted_download(
        "https://objects.githubusercontent.com/github-production-release-asset/file"
    ));
    assert!(policy.trusted_download(
        "https://github.com/lavandiI/lamzu-battery/releases/download/v1/LamzuBattery.exe"
    ));
    assert!(!policy.trusted_download("https://example.com/file"));
    assert_eq!(
        lamzu_battery::updater::http::split_url("https://user:pw@github.com:443/path"),
        Some(("https", "github.com", "/path"))
    );
    assert_eq!(lamzu_battery::updater::http::split_url("nonsense"), None);
}
