//! Crash reporting: a panic must land in `crash.log`.

use std::time::Duration;

#[test]
fn panic_is_written_to_the_crash_log() {
    let directory = tempfile::tempdir().expect("temp dir");
    let crash = directory.path().join("crash.log");
    lamzu_battery::crash::install(&crash);

    let result = std::panic::catch_unwind(|| panic!("probe failure 42"));
    assert!(result.is_err(), "the probe must panic");
    // The hook writes synchronously, but give the file system a moment.
    std::thread::sleep(Duration::from_millis(200));

    let text = std::fs::read_to_string(&crash).expect("crash log");
    assert!(text.contains("[panic]"), "no panic marker: {text}");
    assert!(
        text.contains("probe failure 42"),
        "no panic payload: {text}"
    );
    assert!(text.contains("thread="), "no thread name: {text}");
    assert!(text.contains("crash_report.rs"), "no location: {text}");
}

#[test]
fn oversized_crash_log_is_truncated_at_startup() {
    let directory = tempfile::tempdir().expect("temp dir");
    let crash = directory.path().join("crash.log");
    std::fs::write(&crash, vec![b'x'; 300 * 1024]).expect("write");
    lamzu_battery::crash::install(&crash);
    let length = std::fs::metadata(&crash).expect("metadata").len();
    assert!(
        length < 300 * 1024,
        "the stale log was not removed: {length}"
    );
}
