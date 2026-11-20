//! LAMZU Battery: battery indicator for LAMZU mice in the Windows system tray.
//!
//! The crate is split into a safe core (protocol, i18n, icons, settings, updater)
//! and a thin Win32 translation layer in [`win`], which is the only module allowed
//! to contain `unsafe` code.
#![deny(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
#![cfg_attr(not(windows), allow(dead_code))]

#[cfg(not(windows))]
compile_error!("LAMZU Battery is a Windows-only application");

pub mod app;
pub mod autostart;
pub mod cli;
pub mod config;
pub mod core;
pub mod crash;
pub mod diagnostics;
pub mod hid;
pub mod i18n;
pub mod icons;
pub mod logging;
pub mod notify;
pub mod protocol;
pub mod settings;
pub mod tray;
pub mod tray_visibility;
pub mod updater;
#[expect(
    unsafe_code,
    reason = "Win32 FFI translation layer is isolated here; no other module may use unsafe"
)]
pub mod win;

use std::time::Duration;

/// Product name shown in menus, message boxes and notifications.
pub const APP_NAME: &str = "LAMZU Battery";
/// Registry/executable identifier without spaces.
pub const APP_ID: &str = "LamzuBattery";
/// Application version, kept in sync with the release tag.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Named mutex that keeps a single tray instance alive.
pub const MUTEX_TRAY: &str = "Local\\LamzuBatteryTray";
/// Named mutex that serialises executable replacement and backup cleanup.
pub const MUTEX_UPDATE_FILES: &str = "Local\\LamzuBatteryUpdateFiles";

/// Autostart registry location.
pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// Autostart value written by this application.
pub const RUN_VALUE: &str = "LamzuBattery";
/// Autostart value written by 1.0-1.2 releases.
pub const LEGACY_RUN_VALUE: &str = "LamzuThornBattery";

/// Undocumented Windows 11 notification-area settings key.
pub const NOTIFY_ICON_KEY: &str = r"Control Panel\NotifyIconSettings";
/// Per-icon visibility value inside [`NOTIFY_ICON_KEY`].
pub const NOTIFY_ICON_PROMOTED: &str = "IsPromoted";
/// Executable path value inside [`NOTIFY_ICON_KEY`].
pub const NOTIFY_ICON_EXE: &str = "ExecutablePath";
/// First Windows 11 build that stores per-icon visibility in the registry.
pub const MIN_PROMOTION_BUILD: u32 = 22621;

/// Taskbar theme key.
pub const PERSONALIZE_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
/// Value that tells whether the taskbar is drawn light.
pub const LIGHT_THEME_VALUE: &str = "SystemUsesLightTheme";

/// Release page opened from the tray menu.
pub const RELEASES_URL: &str = "https://github.com/lavandiI/lamzu-battery/releases/latest";
/// GitHub API used by the updater.
pub const API_URL: &str = "https://api.github.com/repos/lavandiI/lamzu-battery/releases/latest";
/// Asset name expected in a release.
pub const ASSET_NAME: &str = "LamzuBattery.exe";
/// User agent for GitHub requests.
pub const USER_AGENT: &str = "LamzuBattery";

/// AppUserModelId used by toast notifications.
pub const APP_USER_MODEL_ID: &str = "LamzuBattery";

/// Normal battery poll interval.
pub const POLL_INTERVAL: Duration = Duration::from_secs(300);
/// Poll interval while charging, so 100% is not missed.
pub const POLL_INTERVAL_CHARGING: Duration = Duration::from_secs(60);
/// Retry interval while the mouse is asleep or unplugged.
pub const POLL_INTERVAL_OFFLINE: Duration = Duration::from_secs(20);
/// How often the presence of cable/dongle interfaces is re-read.
pub const DEVICE_WATCH_INTERVAL: Duration = Duration::from_secs(5);
/// Delay before the instance reports itself healthy after startup.
pub const HEALTH_DELAY: Duration = Duration::from_secs(5);
/// Normal update-check interval.
pub const UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
/// Delay before the first automatic update check.
pub const UPDATE_FIRST_CHECK_DELAY: Duration = Duration::from_secs(8);
/// Retry delays after a failed update check.
pub const UPDATE_RETRY_DELAYS: [Duration; 5] = [
    Duration::from_secs(60),
    Duration::from_secs(300),
    Duration::from_secs(900),
    Duration::from_secs(3600),
    Duration::from_secs(6 * 60 * 60),
];
/// How long the updated instance has to report readiness.
pub const UPDATE_READY_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a rollback copy is kept after a successful start.
pub const BACKUP_GRACE_PERIOD: Duration = Duration::from_secs(60 * 60);
/// Rollback copies older than this are removed on startup.
pub const STALE_BACKUP_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
/// Value above which a stale reading is shown as "?".
pub const STALE_READING: Duration = Duration::from_secs(900);
/// Maximum accepted size of an update download.
pub const MAX_UPDATE_BYTES: u64 = 200 * 1024 * 1024;
/// Maximum accepted size of the GitHub API response.
pub const MAX_JSON_BYTES: u64 = 1024 * 1024;
/// Download chunk size.
pub const DOWNLOAD_CHUNK: usize = 256 * 1024;
/// Total time budget for one download.
pub const DOWNLOAD_DEADLINE: Duration = Duration::from_secs(15 * 60);
/// Retry delays for file replacement.
pub const REPLACE_RETRY_DELAYS: [Duration; 4] = [
    Duration::from_millis(100),
    Duration::from_millis(250),
    Duration::from_millis(500),
    Duration::from_secs(1),
];
/// Delay before a failed automated update error is repeated.
pub const UPDATE_ERROR_NOTIFY_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// Timeout of one HID exchange.
pub const HID_TIMEOUT: Duration = Duration::from_millis(500);
/// Size of the rendered tray icon.
pub const ICON_SIZE: u32 = 64;

/// Returns true in release builds, which is the equivalent of Python's `sys.frozen`.
#[must_use]
pub const fn is_packaged() -> bool {
    !cfg!(debug_assertions)
}
