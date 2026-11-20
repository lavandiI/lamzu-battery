//! Filesystem layout and the portable data directory.

use std::path::{Path, PathBuf};

/// Every path the application uses.
#[derive(Debug, Clone)]
pub struct Paths {
    /// Directory that contains the executable.
    pub base_dir: PathBuf,
    /// Path of the running executable.
    pub exe: PathBuf,
    /// Directory for logs, settings and status.
    pub data_dir: PathBuf,
    /// Last known reading and update health marker.
    pub status: PathBuf,
    /// Language and update mode.
    pub settings: PathBuf,
    /// Rotating log.
    pub log: PathBuf,
    /// Native fault and panic log.
    pub crash: PathBuf,
    /// Portable directory that could not be used, if any.
    pub fallback: Option<PathBuf>,
}

impl Paths {
    /// Discover paths for the running executable.
    #[must_use]
    pub fn discover() -> Self {
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("LamzuBattery.exe"));
        let base = exe
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        Self::for_base(base, exe)
    }

    /// Build the layout for an explicit base directory.
    #[must_use]
    pub fn for_base(base: PathBuf, exe: PathBuf) -> Self {
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
        let temp = std::env::temp_dir();
        let (data_dir, fallback) = pick_data_dir(&base, local.as_deref(), &temp);
        Self {
            base_dir: base,
            exe,
            status: data_dir.join("status.json"),
            settings: data_dir.join("settings.json"),
            log: data_dir.join("app.log"),
            crash: data_dir.join("crash.log"),
            data_dir,
            fallback,
        }
    }
}

/// Prove that a directory is writable by creating and removing a probe file.
///
/// A unique name keeps two instances starting at the same time from deleting
/// each other's probe and wrongly falling back.
#[must_use]
pub fn is_writable_dir(path: &Path) -> bool {
    if std::fs::create_dir_all(path).is_err() {
        return false;
    }
    let probe = path.join(format!(".write_test-{}", std::process::id()));
    if std::fs::write(&probe, b"ok").is_err() {
        return false;
    }
    let _ = std::fs::remove_file(&probe);
    true
}

/// Prefer `<base>/data`, then `%LOCALAPPDATA%/LamzuBattery`, then the temp
/// directory. Returns the chosen directory and the portable one when it was
/// rejected.
#[must_use]
pub fn pick_data_dir(
    base: &Path,
    local_app_data: Option<&Path>,
    temp: &Path,
) -> (PathBuf, Option<PathBuf>) {
    let preferred = base.join("data");
    if is_writable_dir(&preferred) {
        return (preferred, None);
    }
    let mut candidates = Vec::new();
    if let Some(local) = local_app_data {
        candidates.push(local.join(crate::APP_ID));
    }
    candidates.push(temp.join(crate::APP_ID));
    for candidate in candidates {
        if is_writable_dir(&candidate) {
            return (candidate, Some(preferred));
        }
    }
    (preferred.clone(), Some(preferred))
}
