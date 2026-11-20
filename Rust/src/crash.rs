//! Crash reporting: Rust panics and native faults.
//!
//! A windowed process has no stderr, so every panic is appended to
//! `crash.log`; native faults are formatted by the Win32 filter installed in
//! [`crate::win`] and written to the same file. Panics are treated as "this
//! program should stop": release builds abort and the user (or the autostart
//! entry) launches a fresh instance.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::OnceLock;

use crate::i18n::{self, Key};
use crate::win;

/// Maximum size of `crash.log`; larger files are removed at startup.
const MAX_CRASH_BYTES: u64 = 256 * 1024;

static CRASH_FILE: OnceLock<File> = OnceLock::new();

fn append(text: &str) {
    if let Some(file) = CRASH_FILE.get() {
        let mut file = file;
        let _ = file.write_all(text.as_bytes());
        let _ = file.flush();
    }
}

fn payload_text(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string panic payload")
}

fn on_panic(info: &std::panic::PanicHookInfo<'_>) {
    let thread = std::thread::current();
    let name = thread.name().unwrap_or("unnamed");
    let location = info
        .location()
        .map(|location| {
            format!(
                "{}:{}:{}",
                location.file(),
                location.line(),
                location.column()
            )
        })
        .unwrap_or_else(|| "unknown".to_owned());
    let backtrace = std::backtrace::Backtrace::force_capture();
    let text = format!(
        "\n[panic] thread={name} at {location}\npayload: {}\n{backtrace}\n",
        payload_text(info.payload())
    );
    append(&text);
    log::error!(
        "panic in thread {name} at {location}: {}",
        payload_text(info.payload())
    );
    if name == "main" {
        win::message_box(
            i18n::tr(i18n::Language::En, Key::CrashTitle),
            i18n::tr(i18n::Language::En, Key::CrashBody),
            true,
        );
    }
}

/// Install the panic hook and the native fault reporter.
pub fn install(crash_path: &Path) {
    if let Ok(metadata) = std::fs::metadata(crash_path)
        && metadata.len() > MAX_CRASH_BYTES
    {
        let _ = std::fs::remove_file(crash_path);
    }
    if let Some(parent) = crash_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match OpenOptions::new()
        .create(true)
        .append(true)
        .open(crash_path)
    {
        Ok(file) => {
            let _ = CRASH_FILE.set(file);
        }
        Err(error) => log::warn!("could not open {}: {error}", crash_path.display()),
    }

    std::panic::set_hook(Box::new(on_panic));
    win::install_fault_reporter(append);
}
