//! Pin the tray icon to the Windows 11 taskbar.
//!
//! Windows 11 22H2+ keeps per-icon visibility in
//! `HKCU\Control Panel\NotifyIconSettings`. A missing `IsPromoted` value means
//! the icon was never configured (usually hidden in the overflow flyout), so it
//! is set to `1`. An explicit `0` means the user hid the icon and is respected.

use std::path::Path;
use std::time::Duration;

use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE};

use crate::win;

/// One notification-area entry of this executable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayEntry {
    /// Subkey name.
    pub key_name: String,
    /// Executable path recorded by Explorer.
    pub executable: String,
    /// `IsPromoted`, when present.
    pub promoted: Option<u32>,
}

/// Registry access used by the promotion logic.
pub trait NotifyIconRegistry: Send + Sync {
    /// Every entry that records an executable.
    fn entries(&self) -> Vec<TrayEntry>;
    /// Set `IsPromoted = 1` for a subkey.
    fn set_promoted(&self, key_name: &str) -> Result<(), String>;
}

/// Real registry implementation.
#[derive(Debug, Default)]
pub struct WindowsRegistry;

impl NotifyIconRegistry for WindowsRegistry {
    fn entries(&self) -> Vec<TrayEntry> {
        let mut entries = Vec::new();
        let Ok(root) = RegKey::predef(HKEY_CURRENT_USER).open_subkey(crate::NOTIFY_ICON_KEY) else {
            return entries;
        };
        for name in root.enum_keys().flatten() {
            let Ok(key) = root.open_subkey(&name) else {
                continue;
            };
            let Ok(executable) = key.get_value::<String, _>(crate::NOTIFY_ICON_EXE) else {
                continue;
            };
            let promoted = key.get_value::<u32, _>(crate::NOTIFY_ICON_PROMOTED).ok();
            entries.push(TrayEntry {
                key_name: name,
                executable,
                promoted,
            });
        }
        entries
    }

    fn set_promoted(&self, key_name: &str) -> Result<(), String> {
        let path = format!("{}\\{key_name}", crate::NOTIFY_ICON_KEY);
        let key = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(&path, KEY_SET_VALUE)
            .map_err(|error| error.to_string())?;
        key.set_value(crate::NOTIFY_ICON_PROMOTED, &1_u32)
            .map_err(|error| error.to_string())
    }
}

/// Outcome of a promotion attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Promotion {
    /// The icon was promoted by this call.
    Promoted,
    /// The icon was already promoted.
    Already,
    /// The user explicitly hid the icon.
    OptedOut,
    /// The running Windows build does not support the key.
    Unsupported,
    /// No entry for this executable exists.
    NotFound,
    /// The registry could not be written.
    Error,
    /// The caller asked to stop.
    Stopped,
}

const AUTO_RETRY_DELAYS: [Duration; 10] = [
    Duration::from_millis(500),
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(3),
    Duration::from_secs(5),
    Duration::from_secs(8),
    Duration::from_secs(10),
    Duration::from_secs(10),
    Duration::from_secs(10),
    Duration::from_secs(10),
];
const MANUAL_RETRY_DELAYS: [Duration; 4] = [
    Duration::from_millis(500),
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(5),
];

/// Delays used by the automatic promotion after startup.
pub const AUTO_DELAYS: &[Duration] = &AUTO_RETRY_DELAYS;
/// Delays used when the user asks for promotion from the menu.
pub const MANUAL_DELAYS: &[Duration] = &MANUAL_RETRY_DELAYS;

/// Wait for `delay`, returning true when the caller asked to stop.
fn wait(delay: Duration, stop: &dyn Fn() -> bool) -> bool {
    let step = Duration::from_millis(50);
    let mut remaining = delay;
    while remaining > Duration::ZERO {
        if stop() {
            return true;
        }
        std::thread::sleep(step.min(remaining));
        remaining = remaining.saturating_sub(step);
    }
    stop()
}

fn entries_for(registry: &dyn NotifyIconRegistry, exe: &Path) -> Vec<TrayEntry> {
    let target = win::normalize_path(&exe.to_string_lossy());
    registry
        .entries()
        .into_iter()
        .filter(|entry| win::normalize_path(&entry.executable) == target)
        .collect()
}

/// Promote an icon that was never configured, respecting an explicit `0`.
#[must_use]
pub fn promote_if_unconfigured(
    registry: &dyn NotifyIconRegistry,
    exe: &Path,
    build: u32,
    delays: &[Duration],
    stop: &dyn Fn() -> bool,
) -> Promotion {
    if build < crate::MIN_PROMOTION_BUILD {
        return Promotion::Unsupported;
    }
    let mut schedule = vec![Duration::ZERO];
    schedule.extend_from_slice(delays);

    for delay in schedule {
        if wait(delay, stop) {
            return Promotion::Stopped;
        }
        let entries = entries_for(registry, exe);
        if entries.is_empty() {
            continue;
        }
        if entries.iter().any(|entry| entry.promoted == Some(0)) {
            return Promotion::OptedOut;
        }
        let Some(missing) = entries.iter().find(|entry| entry.promoted.is_none()) else {
            return Promotion::Already;
        };
        return match registry.set_promoted(&missing.key_name) {
            Ok(()) => Promotion::Promoted,
            Err(error) => {
                log::debug!("could not promote tray icon: {error}");
                Promotion::Error
            }
        };
    }
    Promotion::NotFound
}

/// Explicit user request: promote every entry of this executable.
#[must_use]
pub fn promote(
    registry: &dyn NotifyIconRegistry,
    exe: &Path,
    build: u32,
    delays: &[Duration],
    stop: &dyn Fn() -> bool,
) -> Promotion {
    if build < crate::MIN_PROMOTION_BUILD {
        return Promotion::Unsupported;
    }
    let mut schedule = vec![Duration::ZERO];
    schedule.extend_from_slice(delays);
    let mut failed = false;

    for delay in schedule {
        if wait(delay, stop) {
            return Promotion::Stopped;
        }
        let entries = entries_for(registry, exe);
        if entries.is_empty() {
            continue;
        }
        let mut error = false;
        for entry in entries {
            if let Err(message) = registry.set_promoted(&entry.key_name) {
                log::debug!("could not promote {}: {message}", entry.key_name);
                error = true;
            }
        }
        if !error {
            return Promotion::Promoted;
        }
        failed = true;
    }
    if failed {
        Promotion::Error
    } else {
        Promotion::NotFound
    }
}

/// Open the taskbar settings page.
#[must_use]
pub fn open_taskbar_settings() -> bool {
    win::open("ms-settings:taskbar")
}
