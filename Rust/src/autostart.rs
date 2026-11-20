//! Autostart through `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.

use std::path::Path;

use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};

/// Command written to the Run key for this executable.
#[must_use]
pub fn command(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

fn same(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}

fn run_key() -> std::io::Result<RegKey> {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(crate::RUN_KEY, KEY_READ | KEY_SET_VALUE)
}

/// Whether autostart currently points at this executable.
#[must_use]
pub fn enabled(exe: &Path) -> bool {
    let Ok(key) = RegKey::predef(HKEY_CURRENT_USER).open_subkey(crate::RUN_KEY) else {
        return false;
    };
    match key.get_value::<String, _>(crate::RUN_VALUE) {
        Ok(value) => same(&value, &command(exe)),
        Err(_) => false,
    }
}

/// Enable or disable autostart.
pub fn set(enabled: bool, exe: &Path) -> std::io::Result<()> {
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(crate::RUN_KEY)?;
    if enabled {
        key.set_value(crate::RUN_VALUE, &command(exe))
    } else {
        match key.delete_value(crate::RUN_VALUE) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

/// Move the value written by 1.0-1.2 releases to the current name.
pub fn migrate_legacy(exe: &Path) {
    let Ok(key) = run_key() else {
        log::debug!("could not open the Run key for autostart migration");
        return;
    };
    if key.get_value::<String, _>(crate::LEGACY_RUN_VALUE).is_err() {
        return;
    }
    if key.get_value::<String, _>(crate::RUN_VALUE).is_err() {
        let _ = key.set_value(crate::RUN_VALUE, &command(exe));
    }
    let _ = key.delete_value(crate::LEGACY_RUN_VALUE);
    log::info!("migrated autostart from {}", crate::LEGACY_RUN_VALUE);
}
