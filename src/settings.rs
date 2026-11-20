//! `settings.json` and `status.json`.
//!
//! Both files use the same layout as the Python implementation, so switching
//! between builds keeps the user's language, update mode and the updater's
//! health marker. Unknown settings keys are preserved on save.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::i18n::Language;

/// How new releases are installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateMode {
    /// Download and install without asking.
    Auto,
    /// Only notify, install from the menu.
    Notify,
}

impl UpdateMode {
    /// Parse the stored value, falling back to [`Self::Auto`].
    #[must_use]
    pub fn from_value(value: Option<&Value>) -> Self {
        match value.and_then(Value::as_str) {
            Some(text) if text.eq_ignore_ascii_case("notify") => Self::Notify,
            _ => Self::Auto,
        }
    }

    /// Value written to `settings.json`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Notify => "notify",
        }
    }
}

/// Current unix time in seconds.
#[must_use]
pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        })
}

fn read_json_map(path: &Path) -> Map<String, Value> {
    let Ok(bytes) = std::fs::read(path) else {
        return Map::new();
    };
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Object(map)) => map,
        _ => Map::new(),
    }
}

fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file_name = path
        .file_name()
        .map_or_else(|| "data".into(), |name| name.to_string_lossy().into_owned());
    let temporary = path.with_file_name(format!(".{file_name}.{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// `settings.json`.
#[derive(Debug, Clone)]
pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    /// Store at `path`.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Raw values, or an empty map when the file is missing or broken.
    #[must_use]
    pub fn values(&self) -> Map<String, Value> {
        read_json_map(&self.path)
    }

    /// Selected language, if one was stored.
    #[must_use]
    pub fn language(&self) -> Option<Language> {
        self.values()
            .get("language")
            .and_then(Value::as_str)
            .map(Language::from_code)
    }

    /// Selected update mode.
    #[must_use]
    pub fn update_mode(&self) -> UpdateMode {
        let values = self.values();
        UpdateMode::from_value(values.get("update_mode"))
    }

    /// Merge one key into the file.
    pub fn set(&self, key: &str, value: Value) -> std::io::Result<()> {
        self.update(|map| {
            map.insert(key.to_owned(), value);
        })
    }

    /// Merge several keys into the file.
    pub fn update(&self, apply: impl FnOnce(&mut Map<String, Value>)) -> std::io::Result<()> {
        let mut values = read_json_map(&self.path);
        apply(&mut values);
        let mode = UpdateMode::from_value(values.get("update_mode"));
        values.insert(
            "update_mode".to_owned(),
            Value::String(mode.as_str().to_owned()),
        );
        let bytes =
            serde_json::to_vec_pretty(&Value::Object(values)).map_err(std::io::Error::other)?;
        write_atomically(&self.path, &bytes)
    }
}

/// Payload of `status.json`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct StatusPayload {
    /// Product name.
    pub app: String,
    /// Application version.
    pub version: String,
    /// Process id.
    pub pid: u32,
    /// Display name shown in the tray.
    pub display_name: String,
    /// Interface language.
    pub language: String,
    /// Detected model.
    pub model: Option<String>,
    /// Detected protocol.
    pub protocol: Option<String>,
    /// Raw product string.
    pub product: Option<String>,
    /// False in the final write during shutdown.
    pub running: bool,
    /// True once the tray has proven that it works.
    pub healthy: bool,
    /// Freshness marker read by the updater.
    pub heartbeat_unix: i64,
    /// True while the last poll succeeded.
    pub online: bool,
    /// Percentage of the last reading.
    pub percent: Option<u8>,
    /// Raw state of the last reading.
    pub state: Option<u8>,
    /// Coarse device level.
    pub raw_percent: Option<u8>,
    /// Battery voltage.
    pub voltage_mv: Option<u16>,
    /// True while charging.
    pub charging: bool,
    /// True when full.
    pub full: bool,
    /// When the last successful reading was taken.
    pub updated_unix: Option<i64>,
    /// When the last poll happened.
    pub polled_unix: i64,
    /// When the last update check happened.
    pub update_last_check_unix: Option<i64>,
    /// Last update error, if any.
    pub update_last_error: Option<String>,
}

/// Health marker read by the updater.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct HealthSnapshot {
    /// Version reported by the new instance.
    #[serde(default)]
    pub version: String,
    /// Whether the new instance reported a working tray.
    #[serde(default)]
    pub healthy: bool,
    /// Freshness marker.
    #[serde(default)]
    pub heartbeat_unix: i64,
}

/// `status.json`.
#[derive(Debug, Clone)]
pub struct StatusStore {
    path: PathBuf,
}

impl StatusStore {
    /// Store at `path`.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Write the payload atomically.
    pub fn write(&self, payload: &StatusPayload) -> std::io::Result<()> {
        let bytes = serde_json::to_vec_pretty(payload).map_err(std::io::Error::other)?;
        write_atomically(&self.path, &bytes)
    }

    /// Read the fields used by the updater's health check.
    #[must_use]
    pub fn health(&self) -> Option<HealthSnapshot> {
        let bytes = std::fs::read(&self.path).ok()?;
        let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
        serde_json::from_slice(bytes).ok()
    }

    /// Raw file contents, or an empty map.
    #[must_use]
    pub fn values(&self) -> Map<String, Value> {
        read_json_map(&self.path)
    }
}
