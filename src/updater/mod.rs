//! Self-update support: version comparison, download verification, executable
//! replacement and rollback.
//!
//! The updater talks only to the official GitHub repository, verifies the
//! SHA-256 digest published by GitHub, keeps a rollback copy of the previous
//! executable and removes it again after a successful start.

pub mod http;

use std::ffi::OsStr;
use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{Duration, Instant, SystemTime};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::settings::HealthSnapshot;
use crate::win;

/// Stage in which an update failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateStage {
    /// Contacting GitHub or parsing the answer.
    Check,
    /// Network transfer.
    Network,
    /// Checking that the executable can be replaced.
    Preflight,
    /// Downloading the asset.
    Download,
    /// Size, digest or executable magic check.
    Verification,
    /// Replacing the executable.
    Install,
    /// Waiting for the new instance to report readiness.
    Health,
}

impl fmt::Display for UpdateStage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Check => "check",
            Self::Network => "network",
            Self::Preflight => "preflight",
            Self::Download => "download",
            Self::Verification => "verification",
            Self::Install => "install",
            Self::Health => "health",
        };
        formatter.write_str(text)
    }
}

/// A failed update step.
#[derive(Debug)]
pub struct UpdateError {
    stage: UpdateStage,
    message: String,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
    #[expect(dead_code, reason = "captured so a future error report can print it")]
    backtrace: std::backtrace::Backtrace,
}

impl UpdateError {
    /// Create an error for a stage.
    #[must_use]
    pub fn new(stage: UpdateStage, message: impl Into<String>) -> Self {
        Self {
            stage,
            message: message.into(),
            source: None,
            backtrace: std::backtrace::Backtrace::capture(),
        }
    }

    /// Network-level failure.
    #[must_use]
    pub fn network(message: impl Into<String>) -> Self {
        Self::new(UpdateStage::Network, message)
    }

    /// Attach the underlying error.
    #[must_use]
    pub fn with_source(mut self, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    /// Stage that failed.
    #[must_use]
    pub const fn stage(&self) -> UpdateStage {
        self.stage
    }

    /// Human readable reason, used in notifications.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for UpdateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.stage, self.message)
    }
}

impl std::error::Error for UpdateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_deref()
            .map(|error| error as &(dyn std::error::Error + 'static))
    }
}

/// A newer release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInfo {
    /// Version without the leading `v`.
    pub version: String,
    /// Download URL of the executable.
    pub asset_url: String,
    /// Expected size in bytes.
    pub asset_size: u64,
    /// Expected lowercase SHA-256 digest.
    pub digest: String,
}

/// A dotted numeric version.
#[derive(Debug, Clone, PartialEq, Eq, Default, PartialOrd, Ord)]
pub struct Version(Vec<u32>);

impl Version {
    /// Parse the first dotted number found in `text`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let start = text.find(|character: char| character.is_ascii_digit())?;
        let mut parts = Vec::new();
        let mut current = String::new();
        for character in text[start..].chars() {
            if character.is_ascii_digit() {
                current.push(character);
            } else if character == '.' && !current.is_empty() {
                parts.push(current.parse().ok()?);
                current.clear();
            } else {
                break;
            }
        }
        if !current.is_empty() {
            parts.push(current.parse().ok()?);
        }
        if parts.is_empty() {
            None
        } else {
            Some(Self(parts))
        }
    }

    fn compare(&self, other: &Self) -> std::cmp::Ordering {
        let width = self.0.len().max(other.0.len());
        (0..width)
            .map(|index| {
                self.0
                    .get(index)
                    .copied()
                    .unwrap_or(0)
                    .cmp(&other.0.get(index).copied().unwrap_or(0))
            })
            .find(|ordering| ordering.is_ne())
            .unwrap_or(std::cmp::Ordering::Equal)
    }

    /// True when `self` is a newer version than `other`.
    #[must_use]
    pub fn is_newer_than(&self, other: &Self) -> bool {
        self.compare(other) == std::cmp::Ordering::Greater
    }

    /// True when both versions describe the same release.
    #[must_use]
    pub fn same_as(&self, other: &Self) -> bool {
        self.compare(other) == std::cmp::Ordering::Equal
    }
}

impl FromStr for Version {
    type Err = ();

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text).ok_or(())
    }
}

impl fmt::Display for Version {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text: Vec<String> = self.0.iter().map(u32::to_string).collect();
        formatter.write_str(&text.join("."))
    }
}

/// True when `candidate` is newer than `current`.
#[must_use]
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (Version::parse(candidate), Version::parse(current)) {
        (Some(candidate), Some(current)) => candidate.is_newer_than(&current),
        _ => false,
    }
}

/// True when both strings describe the same version.
#[must_use]
pub fn same_version(left: &str, right: &str) -> bool {
    match (Version::parse(left), Version::parse(right)) {
        (Some(left), Some(right)) => left.same_as(&right),
        _ => false,
    }
}

/// Absolute path of the running executable, or `None` for development builds.
#[must_use]
pub fn executable_path() -> Option<PathBuf> {
    if !crate::is_packaged() {
        return None;
    }
    std::env::current_exe().ok()
}

/// Download and rollback paths next to `target`.
#[must_use]
pub fn update_paths(target: &Path) -> (PathBuf, PathBuf) {
    let stem = target.file_stem().map_or_else(
        || target.to_string_lossy().into_owned(),
        |stem| stem.to_string_lossy().into_owned(),
    );
    let suffix = target.extension().map_or_else(String::new, |extension| {
        format!(".{}", extension.to_string_lossy())
    });
    (
        target.with_file_name(format!("{stem}.new{suffix}")),
        target.with_file_name(format!("{stem}.old{suffix}")),
    )
}

fn side_path(path: &Path, suffix: &str) -> PathBuf {
    path.with_file_name(format!(
        "{}{}",
        path.file_name()
            .map_or_else(|| "LamzuBattery".into(), OsStr::to_string_lossy),
        suffix
    ))
}

/// Fail before downloading when the executable cannot be replaced safely.
pub fn ensure_update_target(target: &Path, required_bytes: u64) -> Result<(), UpdateError> {
    if !target.is_file() {
        return Err(UpdateError::new(
            UpdateStage::Preflight,
            format!("current exe not found: {}", target.display()),
        ));
    }
    if target
        .metadata()
        .map(|metadata| metadata.permissions().readonly())
        .unwrap_or(false)
    {
        return Err(UpdateError::new(
            UpdateStage::Preflight,
            format!("the current exe is read-only: {}", target.display()),
        ));
    }

    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let probe = parent.join(format!(
        ".{}.write-test-{}",
        target
            .file_stem()
            .map_or_else(|| "LamzuBattery".into(), |stem| stem.to_string_lossy()),
        std::process::id()
    ));
    let write_result = std::fs::write(&probe, b"ok");
    let _ = std::fs::remove_file(&probe);
    if let Err(error) = write_result {
        return Err(UpdateError::new(
            UpdateStage::Preflight,
            format!("update folder is not writable: {error}"),
        ));
    }

    if let Some(free) = win::free_space(parent) {
        let current_size = target.metadata().map_or(0, |metadata| metadata.len());
        let needed = required_bytes
            .saturating_add(current_size)
            .saturating_add(1024 * 1024);
        if free < needed {
            return Err(UpdateError::new(
                UpdateStage::Preflight,
                format!(
                    "not enough free space for the update: {} MiB available, {} MiB required",
                    free / (1024 * 1024),
                    needed / (1024 * 1024)
                ),
            ));
        }
    }
    Ok(())
}

/// Normalize the `digest` field of a release asset.
#[must_use]
pub fn normalize_digest(value: Option<&str>) -> Option<String> {
    let mut text = value?.trim();
    if text.to_ascii_lowercase().starts_with("sha256:") {
        text = &text[7..];
    }
    let valid = text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_hexdigit());
    valid.then(|| text.to_ascii_lowercase())
}

#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    name: Option<String>,
    size: Option<u64>,
    digest: Option<String>,
    browser_download_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: Option<String>,
    assets: Option<Vec<ReleaseAsset>>,
}

/// Ask GitHub for the latest release and return it when it is newer.
pub fn check_for_update(
    transport: &dyn http::HttpTransport,
    policy: &http::UrlPolicy,
    current_version: &str,
    api_url: &str,
) -> Result<Option<UpdateInfo>, UpdateError> {
    let body = transport
        .get_bytes(
            api_url,
            "application/vnd.github+json",
            Duration::from_secs(10),
            crate::MAX_JSON_BYTES,
        )
        .map_err(|error| UpdateError::new(UpdateStage::Check, error.message()))?;
    let release: Release = serde_json::from_slice(&body).map_err(|error| {
        UpdateError::new(
            UpdateStage::Check,
            format!("invalid update response: {error}"),
        )
    })?;

    let tag = release.tag_name.unwrap_or_default();
    if !is_newer(&tag, current_version) {
        return Ok(None);
    }
    let Some(asset) = release
        .assets
        .unwrap_or_default()
        .into_iter()
        .find(|asset| asset.name.as_deref() == Some(crate::ASSET_NAME))
    else {
        return Err(UpdateError::new(
            UpdateStage::Check,
            format!("release has no {}", crate::ASSET_NAME),
        ));
    };
    let asset_url = asset.browser_download_url.unwrap_or_default();
    if !policy.trusted_asset(&asset_url) {
        return Err(UpdateError::new(UpdateStage::Check, "untrusted update URL"));
    }
    let digest = normalize_digest(asset.digest.as_deref()).ok_or_else(|| {
        UpdateError::new(UpdateStage::Check, "release asset has no SHA-256 digest")
    })?;
    let asset_size = asset.size.unwrap_or(0);
    if asset_size == 0 || asset_size > crate::MAX_UPDATE_BYTES {
        return Err(UpdateError::new(UpdateStage::Check, "invalid update size"));
    }
    let version = tag
        .strip_prefix('v')
        .or_else(|| tag.strip_prefix('V'))
        .unwrap_or(&tag)
        .to_owned();
    Ok(Some(UpdateInfo {
        version,
        asset_url,
        asset_size,
        digest,
    }))
}

fn remove_quietly(path: &Path) {
    let _ = std::fs::remove_file(path);
}

fn replace_with_retries(source: &Path, destination: &Path) -> std::io::Result<()> {
    let mut last_error = None;
    for attempt in 0..=crate::REPLACE_RETRY_DELAYS.len() {
        match std::fs::rename(source, destination) {
            Ok(()) => return Ok(()),
            Err(error) => {
                last_error = Some(error);
                if let Some(delay) = crate::REPLACE_RETRY_DELAYS.get(attempt) {
                    std::thread::sleep(*delay);
                }
            }
        }
    }
    Err(last_error.unwrap_or_else(|| std::io::Error::other("could not replace file")))
}

/// Download the release asset, verify size, digest and the `MZ` magic and save
/// it next to the executable.
pub fn download_update(
    transport: &dyn http::HttpTransport,
    policy: &http::UrlPolicy,
    info: &UpdateInfo,
    destination: &Path,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<PathBuf, UpdateError> {
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| UpdateError::new(UpdateStage::Download, error.to_string()))?;
    }
    let part = side_path(destination, ".part");
    remove_quietly(&part);

    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut url = info.asset_url.clone();
    let mut file = std::fs::File::create(&part)
        .map_err(|error| UpdateError::new(UpdateStage::Download, error.to_string()))?;
    let deadline = Instant::now() + crate::DOWNLOAD_DEADLINE;
    let mut failure = None;

    'redirects: for _hop in 0..5 {
        let mut sink = |chunk: &[u8]| -> Result<(), UpdateError> {
            if Instant::now() > deadline {
                return Err(UpdateError::new(
                    UpdateStage::Download,
                    "download timed out",
                ));
            }
            total = total.saturating_add(chunk.len() as u64);
            if total > crate::MAX_UPDATE_BYTES {
                return Err(UpdateError::new(
                    UpdateStage::Download,
                    "update is too large",
                ));
            }
            file.write_all(chunk)
                .map_err(|error| UpdateError::new(UpdateStage::Download, error.to_string()))?;
            hasher.update(chunk);
            progress(total, info.asset_size);
            Ok(())
        };
        let meta = match transport.get_stream(
            &url,
            "application/octet-stream",
            Duration::from_secs(120),
            &mut sink,
        ) {
            Ok(meta) => meta,
            Err(error) => {
                failure = Some(error);
                break 'redirects;
            }
        };
        if (300..400).contains(&meta.status) {
            let Some(location) = meta.location else {
                failure = Some(UpdateError::network("redirect without a location"));
                break 'redirects;
            };
            if !policy.trusted_download(&location) {
                failure = Some(UpdateError::new(
                    UpdateStage::Download,
                    "untrusted download redirect",
                ));
                break 'redirects;
            }
            url = location;
            continue;
        }
        if !(200..300).contains(&meta.status) {
            failure = Some(UpdateError::network(format!(
                "server returned HTTP {}",
                meta.status
            )));
        }
        break;
    }

    drop(file);
    let fail = |error: UpdateError| -> UpdateError {
        remove_quietly(&part);
        error
    };
    if let Some(error) = failure {
        return Err(fail(error));
    }

    let digest = hex(&hasher.finalize());
    if total != info.asset_size {
        return Err(fail(UpdateError::new(
            UpdateStage::Verification,
            format!(
                "update size mismatch: got {total}, expected {}",
                info.asset_size
            ),
        )));
    }
    if digest != info.digest {
        return Err(fail(UpdateError::new(
            UpdateStage::Verification,
            format!(
                "update checksum mismatch: got {digest}, expected {}",
                info.digest
            ),
        )));
    }
    let magic = {
        let mut bytes = [0_u8; 2];
        let mut probe = std::fs::File::open(&part).map_err(|error| {
            fail(UpdateError::new(
                UpdateStage::Verification,
                error.to_string(),
            ))
        })?;
        std::io::Read::read_exact(&mut probe, &mut bytes).map_err(|error| {
            fail(UpdateError::new(
                UpdateStage::Verification,
                error.to_string(),
            ))
        })?;
        bytes
    };
    if magic != *b"MZ" {
        return Err(fail(UpdateError::new(
            UpdateStage::Verification,
            "downloaded file is not a Windows executable",
        )));
    }
    replace_with_retries(&part, destination).map_err(|error| {
        fail(UpdateError::new(
            UpdateStage::Download,
            format!("could not save update: {error}"),
        ))
    })?;
    Ok(destination.to_path_buf())
}

fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

fn update_file_lock(timeout: Duration) -> Result<win::HeldMutex, UpdateError> {
    win::HeldMutex::acquire_waiting(crate::MUTEX_UPDATE_FILES, timeout).ok_or_else(|| {
        UpdateError::new(
            UpdateStage::Install,
            "timed out waiting for update file lock",
        )
    })
}

fn remove_with_retries(path: &Path, attempts: usize, delay: Duration) {
    for attempt in 0..attempts {
        if !path.exists() {
            return;
        }
        if std::fs::remove_file(path).is_ok() {
            return;
        }
        if attempt + 1 < attempts {
            std::thread::sleep(delay);
        }
    }
}

/// Move the running executable aside and put the downloaded one in its place.
pub fn replace_executable(
    target: &Path,
    new_path: &Path,
    backup_path: Option<&Path>,
) -> Result<PathBuf, UpdateError> {
    if !target.is_file() {
        return Err(UpdateError::new(
            UpdateStage::Install,
            format!("current exe not found: {}", target.display()),
        ));
    }
    if !new_path.is_file() {
        return Err(UpdateError::new(
            UpdateStage::Install,
            format!("downloaded exe not found: {}", new_path.display()),
        ));
    }
    let backup = backup_path.map_or_else(|| update_paths(target).1, Path::to_path_buf);
    let _lock = update_file_lock(Duration::from_secs(30))?;

    if backup.exists() {
        let stale = backup
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age >= crate::STALE_BACKUP_MAX_AGE);
        if stale {
            remove_with_retries(&backup, 5, Duration::from_millis(200));
        }
    }
    if backup.exists() {
        return Err(UpdateError::new(
            UpdateStage::Install,
            "a previous update backup is still present; wait for the update cleanup to finish",
        ));
    }

    replace_with_retries(target, &backup).map_err(|error| {
        UpdateError::new(
            UpdateStage::Install,
            format!("could not move current exe: {error}"),
        )
    })?;
    if let Ok(file) = std::fs::File::options().write(true).open(&backup) {
        let _ = file.set_modified(SystemTime::now());
    }
    if let Err(error) = replace_with_retries(new_path, target) {
        let _ = replace_with_retries(&backup, target);
        return Err(UpdateError::new(
            UpdateStage::Install,
            format!("could not install update: {error}"),
        ));
    }
    Ok(backup)
}

/// Restore the previous executable after a failed start.
#[must_use]
pub fn rollback_executable(target: &Path, backup: &Path, new_path: &Path) -> bool {
    if !backup.is_file() {
        return false;
    }
    let Ok(_lock) = update_file_lock(Duration::from_secs(30)) else {
        return false;
    };
    if target.is_file() && std::fs::rename(target, new_path).is_err() {
        return false;
    }
    std::fs::rename(backup, target).is_ok()
}

/// Remove an unfinished download; the rollback copy is never touched.
pub fn cleanup_download_files(target: &Path) {
    let (new_path, _) = update_paths(target);
    remove_with_retries(
        &side_path(&new_path, ".part"),
        20,
        Duration::from_millis(250),
    );
    remove_with_retries(&new_path, 20, Duration::from_millis(250));
}

/// Remove the rollback copy once the previous process released it.
pub fn cleanup_backup(target: &Path) {
    let (_, backup) = update_paths(target);
    remove_with_retries(&backup, 240, Duration::from_millis(500));
}

/// Remove a rollback copy left behind by an update older than a day.
#[must_use]
pub fn cleanup_stale_backup(target: &Path) -> bool {
    let (_, backup) = update_paths(target);
    if !backup.exists() {
        return false;
    }
    let old = backup
        .metadata()
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age >= crate::STALE_BACKUP_MAX_AGE);
    if !old {
        return false;
    }
    remove_with_retries(&backup, 5, Duration::from_millis(200));
    !backup.exists()
}

/// Drop the rollback copy before installing a new release.
#[must_use]
pub fn prepare_update_target(target: &Path, timeout: Duration) -> bool {
    let (_, backup) = update_paths(target);
    remove_with_retries(&backup, 5, Duration::from_millis(200));
    let deadline = Instant::now() + timeout;
    while backup.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(250));
    }
    !backup.exists()
}

/// Health-check predicate for a freshly started instance.
#[must_use]
pub fn health_ok(status: Option<&HealthSnapshot>, version: &str, started_at: SystemTime) -> bool {
    let Some(status) = status else {
        return false;
    };
    if !status.healthy || !same_version(&status.version, version) {
        return false;
    }
    let started = started_at
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        });
    status.heartbeat_unix >= started.saturating_sub(5)
}
