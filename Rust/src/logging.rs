//! Rotating file logger.
//!
//! Writes the same line format as the Python implementation and never blocks a
//! caller: if another thread holds the file lock (for example while the process
//! is crashing), the line is dropped instead of deadlocking.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, Once};

use log::{Level, LevelFilter, Log, Metadata, Record};

use crate::config::Paths;
use crate::win;

/// Maximum size of `app.log` before it is rotated.
const MAX_LOG_BYTES: u64 = 256 * 1024;

#[derive(Debug)]
struct LogState {
    file: Option<File>,
    written: u64,
}

#[derive(Debug)]
struct RotatingLogger {
    path: PathBuf,
    state: Mutex<LogState>,
    skipped: AtomicU64,
}

impl RotatingLogger {
    fn open(path: PathBuf) -> Self {
        let (file, written) = match OpenOptions::new().create(true).append(true).open(&path) {
            Ok(file) => {
                let written = file.metadata().map_or(0, |meta| meta.len());
                (Some(file), written)
            }
            Err(_) => (None, 0),
        };
        Self {
            path,
            state: Mutex::new(LogState { file, written }),
            skipped: AtomicU64::new(0),
        }
    }

    fn rotate(&self, state: &mut LogState) {
        state.file = None;
        let backup = self.path.with_extension("log.1");
        let _ = std::fs::remove_file(&backup);
        let _ = std::fs::rename(&self.path, &backup);
        state.written = 0;
        state.file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .ok();
    }
}

impl Log for RotatingLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= Level::Trace
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let Ok(mut state) = self.state.try_lock() else {
            self.skipped.fetch_add(1, Ordering::Relaxed);
            return;
        };
        if state.file.is_none() {
            return;
        }
        let line = format!(
            "{} {:<5} {}\n",
            win::local_timestamp(),
            record.level(),
            record.args()
        );
        let length = u64::try_from(line.len()).unwrap_or(0);
        if state.written + length > MAX_LOG_BYTES {
            self.rotate(&mut state);
        }
        if let Some(file) = state.file.as_mut()
            && file.write_all(line.as_bytes()).is_ok()
        {
            state.written += length;
        }
    }

    fn flush(&self) {
        if let Ok(mut state) = self.state.try_lock()
            && let Some(file) = state.file.as_mut()
        {
            let _ = file.flush();
        }
    }
}

fn try_open(path: &Path) -> bool {
    if let Some(parent) = path.parent()
        && std::fs::create_dir_all(parent).is_err()
    {
        return false;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .is_ok()
}

static INSTALL: Once = Once::new();

/// Configure process-wide logging.
///
/// Returns the path that is actually in use. A read-only data directory falls
/// back to `%TEMP%`; when nothing is writable logging is disabled and the
/// application keeps running.
pub fn init(paths: &Paths, debug: bool) -> Option<PathBuf> {
    let mut chosen = None;
    if try_open(&paths.log) {
        chosen = Some(paths.log.clone());
    } else {
        let fallback = std::env::temp_dir().join(format!("{}.log", crate::APP_ID));
        if try_open(&fallback) {
            chosen = Some(fallback);
        }
    }
    let path = chosen.clone()?;

    INSTALL.call_once(|| {
        let logger = Box::leak(Box::new(RotatingLogger::open(path.clone())));
        if log::set_logger(logger).is_err() {
            log::warn!("logger already installed");
        }
        log::set_max_level(if debug {
            LevelFilter::Debug
        } else {
            LevelFilter::Info
        });
    });

    if chosen.as_ref() != Some(&paths.log) {
        log::warn!(
            "could not write {}, logging to {}",
            paths.log.display(),
            path.display()
        );
    }
    chosen
}

/// Whether a console is attached and console logging makes sense.
pub fn console_available() -> bool {
    win::attach_parent_console()
}
