//! Taskbar promotion decisions and path normalization.

use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use lamzu_battery::tray_visibility::{
    NotifyIconRegistry, Promotion, TrayEntry, promote, promote_if_unconfigured,
};

const BUILD: u32 = 22_631;
const EXE: &str = r"C:\Tools\Lamzu.exe";
const NO_DELAYS: &[Duration] = &[];

struct FakeRegistry {
    entries: Vec<TrayEntry>,
    calls: Mutex<Vec<String>>,
    fail: bool,
}

impl FakeRegistry {
    fn new(entries: Vec<TrayEntry>) -> Self {
        Self {
            entries,
            calls: Mutex::new(Vec::new()),
            fail: false,
        }
    }

    fn failing(entries: Vec<TrayEntry>) -> Self {
        Self {
            entries,
            calls: Mutex::new(Vec::new()),
            fail: true,
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("calls").clone()
    }
}

impl NotifyIconRegistry for FakeRegistry {
    fn entries(&self) -> Vec<TrayEntry> {
        self.entries.clone()
    }

    fn set_promoted(&self, key_name: &str) -> Result<(), String> {
        if self.fail {
            return Err("access denied".to_owned());
        }
        self.calls.lock().expect("calls").push(key_name.to_owned());
        Ok(())
    }
}

fn entry(key: &str, promoted: Option<u32>) -> TrayEntry {
    TrayEntry {
        key_name: key.to_owned(),
        executable: EXE.to_owned(),
        promoted,
    }
}

fn never() -> bool {
    false
}

#[test]
fn missing_value_is_promoted() {
    let registry = FakeRegistry::new(vec![entry("entry", None)]);
    assert_eq!(
        promote_if_unconfigured(&registry, Path::new(EXE), BUILD, NO_DELAYS, &never),
        Promotion::Promoted
    );
    assert_eq!(registry.calls(), vec!["entry".to_owned()]);
}

#[test]
fn explicit_zero_is_respected() {
    let registry = FakeRegistry::new(vec![entry("entry", Some(0))]);
    assert_eq!(
        promote_if_unconfigured(&registry, Path::new(EXE), BUILD, NO_DELAYS, &never),
        Promotion::OptedOut
    );
    assert!(registry.calls().is_empty());
}

#[test]
fn already_promoted_is_a_noop() {
    let registry = FakeRegistry::new(vec![entry("entry", Some(1))]);
    assert_eq!(
        promote_if_unconfigured(&registry, Path::new(EXE), BUILD, NO_DELAYS, &never),
        Promotion::Already
    );
    assert!(registry.calls().is_empty());
}

#[test]
fn zero_wins_over_a_duplicate_entry() {
    let registry = FakeRegistry::new(vec![entry("hidden", Some(0)), entry("new", None)]);
    assert_eq!(
        promote_if_unconfigured(&registry, Path::new(EXE), BUILD, NO_DELAYS, &never),
        Promotion::OptedOut
    );
    assert!(registry.calls().is_empty());
}

#[test]
fn missing_entries_report_not_found() {
    let registry = FakeRegistry::new(Vec::new());
    assert_eq!(
        promote_if_unconfigured(&registry, Path::new(EXE), BUILD, NO_DELAYS, &never),
        Promotion::NotFound
    );
}

#[test]
fn other_executables_are_ignored() {
    let registry = FakeRegistry::new(vec![TrayEntry {
        key_name: "other".to_owned(),
        executable: r"C:\Other\Lamzu.exe".to_owned(),
        promoted: None,
    }]);
    assert_eq!(
        promote_if_unconfigured(&registry, Path::new(EXE), BUILD, NO_DELAYS, &never),
        Promotion::NotFound
    );
}

#[test]
fn unsupported_windows_is_a_noop() {
    let registry = FakeRegistry::new(vec![entry("entry", None)]);
    assert_eq!(
        promote_if_unconfigured(&registry, Path::new(EXE), 19_045, NO_DELAYS, &never),
        Promotion::Unsupported
    );
    assert_eq!(
        promote(&registry, Path::new(EXE), 19_045, NO_DELAYS, &never),
        Promotion::Unsupported
    );
}

#[test]
fn manual_promotion_overrides_zero() {
    let registry = FakeRegistry::new(vec![entry("entry", Some(0))]);
    assert_eq!(
        promote(&registry, Path::new(EXE), BUILD, NO_DELAYS, &never),
        Promotion::Promoted
    );
    assert_eq!(registry.calls(), vec!["entry".to_owned()]);
}

#[test]
fn manual_promotion_reports_a_persistent_error() {
    let registry = FakeRegistry::failing(vec![entry("entry", None)]);
    assert_eq!(
        promote(&registry, Path::new(EXE), BUILD, NO_DELAYS, &never),
        Promotion::Error
    );
}

#[test]
fn promotion_stops_when_asked() {
    let registry = FakeRegistry::new(vec![entry("entry", None)]);
    assert_eq!(
        promote(&registry, Path::new(EXE), BUILD, NO_DELAYS, &|| true),
        Promotion::Stopped
    );
}

#[test]
fn registry_paths_are_compared_like_windows_does() {
    let normalized = lamzu_battery::win::normalize_path(r#" "C:\Tools\Lamzu.exe" "#);
    assert_eq!(normalized, r"c:\tools\lamzu.exe");
    assert_eq!(
        lamzu_battery::win::normalize_path(r"C:/Tools/Lamzu.exe"),
        r"c:\tools\lamzu.exe"
    );
    assert_eq!(lamzu_battery::win::normalize_path(""), "");
}

#[test]
fn environment_variables_are_expanded() {
    let windows = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".to_owned());
    let expanded = lamzu_battery::win::expand_env(r"%WINDIR%\Fonts");
    assert_eq!(expanded, format!(r"{windows}\Fonts"));
    assert_eq!(
        lamzu_battery::win::expand_env("%LAMZU_UNKNOWN_VAR%"),
        "%LAMZU_UNKNOWN_VAR%"
    );
}
