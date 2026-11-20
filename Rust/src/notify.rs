//! Desktop notifications.
//!
//! Windows 10 and 11 render legacy balloons as toasts anyway, so this module
//! talks to the WinRT toast API directly. The AppUserModelId is registered in
//! `HKCU` so the toast is attributed to LAMZU Battery instead of PowerShell.

use std::path::Path;

use winreg::RegKey;
use winreg::enums::HKEY_CURRENT_USER;

use crate::win;

/// Something that can show a notification.
pub trait Notifier: Send + Sync {
    /// Show `title` and `body`; returns false when the notification was not
    /// delivered, so the caller can retry on the next poll.
    fn notify(&self, title: &str, body: &str) -> bool;
}

/// Notifier used by tests and by configurations without a shell.
#[derive(Debug, Default)]
pub struct NullNotifier;

impl Notifier for NullNotifier {
    fn notify(&self, _title: &str, _body: &str) -> bool {
        false
    }
}

/// WinRT toast notifier.
#[derive(Debug)]
pub struct ToastNotifier {
    app_id: String,
}

impl ToastNotifier {
    /// Create a notifier for the application's AppUserModelId.
    #[must_use]
    pub fn new() -> Self {
        Self {
            app_id: crate::APP_USER_MODEL_ID.to_owned(),
        }
    }

    /// Register `DisplayName` and `IconUri` for the AppUserModelId.
    pub fn register_app_id(exe: &Path) {
        let path = format!(
            r"Software\Classes\AppUserModelId\{}",
            crate::APP_USER_MODEL_ID
        );
        let Ok((key, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(&path) else {
            log::debug!("could not register the AppUserModelId");
            return;
        };
        let _ = key.set_value("DisplayName", &crate::APP_NAME);
        let _ = key.set_value("IconUri", &exe.to_string_lossy().into_owned());
    }
}

impl Default for ToastNotifier {
    fn default() -> Self {
        Self::new()
    }
}

impl Notifier for ToastNotifier {
    fn notify(&self, title: &str, body: &str) -> bool {
        if !win::init_com_mta() {
            log::debug!("COM is not available; notification skipped");
            return false;
        }
        let toast = tauri_winrt_notification::Toast::new(&self.app_id)
            .title(title)
            .text1(body);
        match toast.show() {
            Ok(()) => true,
            Err(error) => {
                log::debug!("notification failed: {error}");
                false
            }
        }
    }
}
