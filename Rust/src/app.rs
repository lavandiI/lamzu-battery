//! Application runtime: worker threads, menu actions, status file and updates.
//!
//! Threads are deliberately simple: the poll thread owns the battery cadence and
//! the device watcher, the update thread owns the retry backoff, the health
//! thread writes the readiness marker and removes the rollback copy, and the
//! action thread performs blocking menu work. All of them talk to the tray over
//! channels; the tray itself lives on the main thread.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use serde_json::Value;
use winreg::RegKey;
use winreg::enums::HKEY_CURRENT_USER;

use crate::autostart;
use crate::config::Paths;
use crate::core::{self, Notifications};
use crate::hid::{HidPort, NativeHid, NoHid};
use crate::i18n::{self, Key, Language};
use crate::notify::{Notifier, ToastNotifier};
use crate::protocol::{self, BatteryReading};
use crate::settings::{SettingsStore, StatusPayload, StatusStore, UpdateMode, unix_now};
use crate::tray::{self, MenuAction, UiCommand, UiSnapshot};
use crate::tray_visibility::{self, WindowsRegistry};
use crate::updater::http::{UreqTransport, UrlPolicy};
use crate::updater::{self, UpdateError, UpdateInfo, UpdateStage};
use crate::win::{self, HeldMutex};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Default, Clone)]
struct SharedState {
    last: Option<BatteryReading>,
    last_ok: Option<SystemTime>,
    online: bool,
}

#[derive(Debug, Default)]
struct UpdateState {
    available: Option<UpdateInfo>,
    last_error: Option<String>,
    last_check: Option<i64>,
    notified_version: Option<String>,
    last_error_at: Option<Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PollCommand {
    PollNow,
    RefreshSnapshot,
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdateCommand {
    CheckNow,
    Stop,
}

/// Everything a worker thread needs.
#[derive(Clone)]
struct Runtime {
    paths: Paths,
    settings: SettingsStore,
    status: StatusStore,
    hid: Arc<dyn HidPort>,
    notifier: Arc<dyn Notifier>,
    transport: UreqTransport,
    policy: UrlPolicy,
    language: Arc<Mutex<Language>>,
    shared: Arc<Mutex<SharedState>>,
    updates: Arc<Mutex<UpdateState>>,
    update_mode: Arc<Mutex<UpdateMode>>,
    stop: Arc<AtomicBool>,
    handed_off: Arc<AtomicBool>,
    update_pending: Arc<AtomicBool>,
    tray_ready: Arc<AtomicBool>,
    healthy: Arc<AtomicBool>,
    ui: Sender<UiCommand>,
    ui_thread: Arc<AtomicU32>,
    poll: Sender<PollCommand>,
    update: Sender<UpdateCommand>,
    mutex: Arc<Mutex<Option<HeldMutex>>>,
    exe: Option<PathBuf>,
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let mut truncated: String = text.chars().take(limit.saturating_sub(3)).collect();
    truncated.push_str("...");
    truncated
}

fn taskbar_uses_light_theme() -> bool {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(crate::PERSONALIZE_KEY)
        .ok()
        .and_then(|key| key.get_value::<u32, _>(crate::LIGHT_THEME_VALUE).ok())
        .is_some_and(|value| value == 1)
}

fn spawn_instance(target: &Path) -> std::io::Result<Child> {
    use std::os::windows::process::CommandExt;
    Command::new(target)
        .arg("--wait-for-instance")
        .current_dir(target.parent().unwrap_or_else(|| Path::new(".")))
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
}

fn stop_child(child: &mut Child) -> bool {
    if let Ok(Some(_status)) = child.try_wait() {
        return true;
    }
    let _ = child.kill();
    child.wait().is_ok()
}

fn start(name: &str, task: impl FnOnce() + Send + 'static) -> Option<std::thread::JoinHandle<()>> {
    match std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(task)
    {
        Ok(handle) => Some(handle),
        Err(error) => {
            log::error!("could not start {name}: {error}");
            None
        }
    }
}

impl Runtime {
    fn language(&self) -> Language {
        *self
            .language
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn update_mode(&self) -> UpdateMode {
        *self
            .update_mode
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn wake(&self) {
        let thread_id = self.ui_thread.load(Ordering::Relaxed);
        if thread_id != 0 {
            win::wake_thread(thread_id);
        }
    }

    fn shared_snapshot(&self) -> (Option<BatteryReading>, bool, Duration) {
        let state = self
            .shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let age = state
            .last_ok
            .and_then(|time| SystemTime::now().duration_since(time).ok())
            .unwrap_or_default();
        (state.last.clone(), state.online, age)
    }

    fn available_version(&self) -> Option<String> {
        self.updates
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .available
            .as_ref()
            .map(|info| info.version.clone())
    }

    fn build_snapshot(&self) -> UiSnapshot {
        let (last, online, age) = self.shared_snapshot();
        let language = self.language();
        let light = taskbar_uses_light_theme();
        let view = core::icon_view(last.as_ref(), age, online, light);
        let name = core::display_name(last.as_ref());
        UiSnapshot {
            display_name: name.clone(),
            status: core::status_line(language, last.as_ref(), online, age),
            tooltip: core::tooltip(language, &name, last.as_ref(), online, age),
            icon_text: view.text,
            icon_color: view.color,
            autostart: self.exe.as_ref().is_some_and(|exe| autostart::enabled(exe)),
            auto_update: self.update_mode() == UpdateMode::Auto,
            available_version: self.available_version(),
            language,
        }
    }

    fn send_snapshot(&self) {
        let _ = self.ui.send(UiCommand::Snapshot(self.build_snapshot()));
        self.wake();
    }

    fn send_tooltip(&self, text: String) {
        let _ = self.ui.send(UiCommand::Tooltip(text));
        self.wake();
    }

    fn notify(&self, title: &str, body: impl AsRef<str>) -> bool {
        self.notifier.notify(title, body.as_ref())
    }

    fn write_status(&self, running: bool) {
        if self.handed_off.load(Ordering::SeqCst) || self.update_pending.load(Ordering::SeqCst) {
            return;
        }
        if running && self.stop.load(Ordering::SeqCst) {
            return;
        }
        let state = self
            .shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let (last_check, last_error) = {
            let updates = self
                .updates
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (updates.last_check, updates.last_error.clone())
        };
        let updated = state
            .last_ok
            .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX));
        let payload = StatusPayload {
            app: crate::APP_NAME.to_owned(),
            version: crate::VERSION.to_owned(),
            pid: std::process::id(),
            display_name: core::display_name(state.last.as_ref()),
            language: self.language().code().to_owned(),
            model: state
                .last
                .as_ref()
                .and_then(|reading| reading.model.clone()),
            protocol: state
                .last
                .as_ref()
                .map(|reading| reading.protocol.as_str().to_owned()),
            product: state
                .last
                .as_ref()
                .and_then(|reading| reading.product.clone()),
            running,
            healthy: self.healthy.load(Ordering::SeqCst),
            heartbeat_unix: unix_now(),
            online: state.online,
            percent: state.last.as_ref().map(|reading| reading.percent.get()),
            state: state.last.as_ref().map(|reading| reading.state.raw()),
            raw_percent: state
                .last
                .as_ref()
                .and_then(|reading| reading.raw_percent)
                .map(|percent| percent.get()),
            voltage_mv: state.last.as_ref().and_then(|reading| reading.voltage_mv),
            charging: state.last.as_ref().is_some_and(BatteryReading::charging),
            full: state.last.as_ref().is_some_and(BatteryReading::full),
            updated_unix: updated,
            polled_unix: unix_now(),
            update_last_check_unix: last_check,
            update_last_error: last_error,
        };
        if let Err(error) = self.status.write(&payload) {
            log::debug!("could not write status file: {error}");
        }
    }

    fn set_language(&self, language: Language) {
        *self
            .language
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = language;
        if let Err(error) = self
            .settings
            .set("language", Value::String(language.code().to_owned()))
        {
            log::warn!("could not save the language: {error}");
        }
        log::info!("language = {}", language.code());
        self.write_status(true);
        let _ = self.poll.send(PollCommand::RefreshSnapshot);
    }

    fn refresh_snapshot(&self) {
        let _ = self.poll.send(PollCommand::RefreshSnapshot);
    }

    fn release_mutex(&self) {
        let mut guard = self
            .mutex
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if guard.take().is_some() {
            log::info!("released the tray mutex for the next instance");
        }
    }

    fn reacquire_mutex(&self, timeout: Duration) -> bool {
        let mut guard = self
            .mutex
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if guard.is_some() {
            return true;
        }
        match HeldMutex::acquire_waiting(crate::MUTEX_TRAY, timeout) {
            Some(mutex) => {
                *guard = Some(mutex);
                true
            }
            None => {
                log::error!("could not reacquire the tray mutex after a failed update");
                false
            }
        }
    }

    fn show_quit(&self) {
        let _ = self.ui.send(UiCommand::Quit);
        self.wake();
    }

    fn handle_action(&self, action: MenuAction) -> bool {
        match action {
            MenuAction::Refresh => {
                let _ = self.poll.send(PollCommand::PollNow);
                false
            }
            MenuAction::ToggleAutostart => {
                let language = self.language();
                let Some(exe) = self.exe.clone() else {
                    self.notify(
                        i18n::tr(language, Key::TrayTitle),
                        i18n::tr(language, Key::TraySourceMode),
                    );
                    return false;
                };
                let enable = !autostart::enabled(&exe);
                match autostart::set(enable, &exe) {
                    Ok(()) => log::info!("autostart = {enable}"),
                    Err(error) => log::error!("could not change autostart: {error}"),
                }
                self.refresh_snapshot();
                false
            }
            MenuAction::OpenDataFolder => {
                if !win::open(&self.paths.data_dir.to_string_lossy()) {
                    log::error!("could not open the data folder");
                }
                false
            }
            MenuAction::ShowTrayIcon => {
                let runtime = self.clone();
                let _ = start("tray-promoter", move || runtime.show_tray_icon());
                false
            }
            MenuAction::OpenReleasePage => {
                if !win::open(crate::RELEASES_URL) {
                    log::error!("could not open the release page");
                }
                false
            }
            MenuAction::ToggleAutoUpdate => {
                let mode = if self.update_mode() == UpdateMode::Auto {
                    UpdateMode::Notify
                } else {
                    UpdateMode::Auto
                };
                *self
                    .update_mode
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = mode;
                if let Err(error) = self
                    .settings
                    .set("update_mode", Value::String(mode.as_str().to_owned()))
                {
                    log::warn!("could not save the update mode: {error}");
                }
                log::info!("automatic updates = {}", mode == UpdateMode::Auto);
                self.refresh_snapshot();
                false
            }
            MenuAction::CheckUpdates | MenuAction::InstallUpdate => {
                let _ = self.update.send(UpdateCommand::CheckNow);
                false
            }
            MenuAction::SetLanguage(language) => {
                self.set_language(language);
                false
            }
            MenuAction::Quit => {
                self.stop.store(true, Ordering::SeqCst);
                let _ = self.poll.send(PollCommand::Stop);
                let _ = self.update.send(UpdateCommand::Stop);
                self.show_quit();
                true
            }
        }
    }

    fn show_tray_icon(&self) {
        let language = self.language();
        let Some(exe) = self.exe.clone() else {
            self.notify(
                i18n::tr(language, Key::TrayTitle),
                i18n::tr(language, Key::TraySourceMode),
            );
            return;
        };
        let registry = WindowsRegistry;
        let build = win::build_number();
        let result = tray_visibility::promote(
            &registry,
            &exe,
            build,
            tray_visibility::MANUAL_DELAYS,
            &|| self.stop.load(Ordering::Relaxed),
        );
        log::info!("tray promotion (manual): {result:?}");
        match result {
            tray_visibility::Promotion::Promoted | tray_visibility::Promotion::Already => {
                self.notify(
                    i18n::tr(language, Key::TrayTitle),
                    i18n::tr(language, Key::TrayPinnedBody),
                );
            }
            _ => {
                self.notify(
                    i18n::tr(language, Key::TrayTitle),
                    i18n::tr(language, Key::TrayManualBody),
                );
                let _ = tray_visibility::open_taskbar_settings();
            }
        }
    }

    fn maybe_notify(&self, notifications: &mut Notifications) {
        let ready = self.tray_ready.load(Ordering::Relaxed);
        let (last, _online, _age) = self.shared_snapshot();
        let Some(reading) = last else {
            return;
        };
        let language = self.language();
        let name = core::display_name(Some(&reading));
        if let Some(kind) = notifications.observe(&reading, ready) {
            let (title, body) = match kind {
                core::NotificationKind::Low => (
                    i18n::tr(language, Key::NotifyLowTitle),
                    i18n::tr_args(
                        language,
                        Key::NotifyLowBody,
                        &[("name", &name), ("percent", &reading.percent.to_string())],
                    ),
                ),
                core::NotificationKind::Full => (
                    i18n::tr(language, Key::NotifyFullTitle),
                    i18n::tr_args(
                        language,
                        Key::NotifyFullBody,
                        &[("name", &name), ("percent", &reading.percent.to_string())],
                    ),
                ),
            };
            let delivered = self.notify(title, &body);
            notifications.delivered(kind, delivered);
        }
    }

    fn record_update_error(
        &self,
        error: &UpdateError,
        manual: bool,
        stage: UpdateStage,
        reason: Option<&str>,
    ) {
        let now = Instant::now();
        {
            let mut updates = self
                .updates
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !manual
                && stage == UpdateStage::Check
                && updates.last_error_at.is_some_and(|instant| {
                    now.duration_since(instant) < crate::UPDATE_ERROR_NOTIFY_INTERVAL
                })
            {
                return;
            }
            updates.last_error_at = Some(now);
            updates.last_error = Some(truncate(error.message(), 180));
        }
        let language = self.language();
        let detail = reason.map_or_else(|| truncate(error.message(), 180), str::to_owned);
        let body = if detail.is_empty() {
            i18n::tr(language, Key::UpdateFailed).to_owned()
        } else {
            i18n::tr_args(language, Key::UpdateFailedReason, &[("reason", &detail)])
        };
        self.notify(i18n::tr(language, Key::UpdateTitle), &body);
    }

    fn available_update(&self, info: Option<UpdateInfo>) {
        {
            let mut updates = self
                .updates
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            updates.available = info;
        }
        self.refresh_snapshot();
    }

    fn wait_for_instance(&self, child: &mut Child, version: &str, started_at: SystemTime) -> bool {
        let deadline = Instant::now() + crate::UPDATE_READY_TIMEOUT;
        while Instant::now() < deadline {
            if self.stop.load(Ordering::SeqCst) {
                // The user asked to quit while the update was starting; let the
                // new instance take over instead of rolling it back.
                return true;
            }
            if let Ok(Some(status)) = child.try_wait() {
                log::warn!("updated instance exited early with {status}");
                return false;
            }
            if updater::health_ok(self.status.health().as_ref(), version, started_at) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(400));
        }
        log::warn!("updated instance did not report readiness in time");
        false
    }

    fn check_updates(&self, manual: bool) -> bool {
        let language = self.language();
        let Some(target) = self.exe.clone() else {
            log::debug!("update check skipped: running from source");
            if manual {
                self.notify(
                    i18n::tr(language, Key::UpdateTitle),
                    i18n::tr(language, Key::UpdateSourceMode),
                );
            }
            return true;
        };

        if manual {
            self.notify(
                i18n::tr(language, Key::UpdateTitle),
                i18n::tr(language, Key::UpdateChecking),
            );
        }
        {
            let mut updates = self
                .updates
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            updates.last_check = Some(unix_now());
        }

        let info = match updater::check_for_update(
            &self.transport,
            &self.policy,
            crate::VERSION,
            crate::API_URL,
        ) {
            Ok(info) => info,
            Err(error) => {
                log::warn!("update check failed: {error}");
                self.record_update_error(&error, manual, error.stage(), None);
                return false;
            }
        };
        {
            let mut updates = self
                .updates
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            updates.last_error = None;
        }

        let Some(info) = info else {
            self.available_update(None);
            if manual {
                self.notify(
                    i18n::tr(language, Key::UpdateTitle),
                    i18n::tr_args(language, Key::UpdateLatest, &[("version", crate::VERSION)]),
                );
            }
            return true;
        };

        log::info!(
            "update available: {} (current {})",
            info.version,
            crate::VERSION
        );
        self.available_update(Some(info.clone()));

        if !manual && self.update_mode() == UpdateMode::Notify {
            let first_offer = {
                let mut updates = self
                    .updates
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if updates.notified_version.as_deref() == Some(info.version.as_str()) {
                    false
                } else {
                    updates.notified_version = Some(info.version.clone());
                    true
                }
            };
            if first_offer {
                self.notify(
                    i18n::tr(language, Key::UpdateTitle),
                    i18n::tr_args(
                        language,
                        Key::UpdateAvailable,
                        &[("version", &info.version)],
                    ),
                );
            }
            return true;
        }

        let (new_path, backup_path) = updater::update_paths(&target);
        if !updater::prepare_update_target(&target, Duration::from_secs(10)) {
            let error = UpdateError::new(
                UpdateStage::Preflight,
                "the previous update backup is still being cleaned up",
            );
            log::warn!("update delayed: {error}");
            self.record_update_error(&error, manual, UpdateStage::Preflight, None);
            return false;
        }
        if let Err(error) = updater::ensure_update_target(&target, info.asset_size) {
            log::warn!("update preflight failed: {error}");
            self.record_update_error(&error, manual, UpdateStage::Preflight, None);
            return false;
        }

        let mut progress = |done: u64, total: u64| {
            let percent = (total > 0).then(|| {
                u8::try_from(done.saturating_mul(100) / total)
                    .unwrap_or(100)
                    .min(100)
            });
            let text = match percent {
                Some(percent) => i18n::tr_args(
                    language,
                    Key::UpdateDownloading,
                    &[("percent", &percent.to_string())],
                ),
                None => i18n::tr(language, Key::UpdateDownloadingUnknown).to_owned(),
            };
            self.send_tooltip(format!("{}: {text}", crate::APP_NAME));
        };
        if let Err(error) = updater::download_update(
            &self.transport,
            &self.policy,
            &info,
            &new_path,
            &mut progress,
        ) {
            log::warn!("update download failed: {error}");
            self.send_snapshot();
            self.record_update_error(&error, manual, UpdateStage::Download, None);
            return false;
        }
        self.send_snapshot();
        if self.stop.load(Ordering::SeqCst) {
            log::info!("update cancelled: the application is stopping");
            return true;
        }

        self.update_pending.store(true, Ordering::SeqCst);
        if let Err(error) = updater::replace_executable(&target, &new_path, Some(&backup_path)) {
            self.update_pending.store(false, Ordering::SeqCst);
            log::warn!("update replacement failed: {error}");
            let reason = if target.is_file() {
                None
            } else if updater::rollback_executable(&target, &backup_path, &new_path) {
                log::info!("restored the previous version after a failed replacement");
                Some("the update could not be installed; the previous version was restored")
            } else {
                log::error!("the executable is missing and the backup could not be restored");
                Some(
                    "the update failed and LamzuBattery.exe is missing; download the release from GitHub",
                )
            };
            self.record_update_error(&error, manual, UpdateStage::Install, reason);
            return false;
        }

        log::info!("update installed, restarting {}", target.display());
        let started_at = SystemTime::now();
        let mut child = match spawn_instance(&target) {
            Ok(child) => child,
            Err(error) => {
                log::error!("could not start the updated application: {error}");
                let restored = updater::rollback_executable(&target, &backup_path, &new_path);
                self.update_pending.store(false, Ordering::SeqCst);
                let reason = if restored {
                    "the updated version could not be started; the previous version was restored"
                } else {
                    "the updated version could not be started and the previous version could not be restored; download the release from GitHub"
                };
                let failure = UpdateError::new(UpdateStage::Install, error.to_string());
                self.record_update_error(&failure, manual, UpdateStage::Install, Some(reason));
                return false;
            }
        };

        self.notify(
            i18n::tr(language, Key::UpdateTitle),
            i18n::tr_args(
                language,
                Key::UpdateInstalling,
                &[("version", &info.version)],
            ),
        );
        // The old mutex must be released before the new process can start its
        // tray; the old process stays alive as a short watchdog.
        self.release_mutex();
        if self.wait_for_instance(&mut child, &info.version, started_at) {
            log::info!("updated instance is healthy, stopping the old one");
            self.handed_off.store(true, Ordering::SeqCst);
            self.stop.store(true, Ordering::SeqCst);
            self.show_quit();
            return true;
        }

        log::error!("updated instance failed the health check; rolling back");
        let _stopped = stop_child(&mut child);
        if !self.reacquire_mutex(Duration::from_secs(10)) {
            self.update_pending.store(false, Ordering::SeqCst);
            log::error!("updated instance still owns the tray mutex; files left intact");
            let error = UpdateError::new(
                UpdateStage::Health,
                "the updated instance could not be stopped; restart LAMZU Battery to finish the update",
            );
            self.record_update_error(&error, manual, UpdateStage::Health, None);
            return true;
        }

        let restored = updater::rollback_executable(&target, &backup_path, &new_path);
        self.update_pending.store(false, Ordering::SeqCst);
        let reason = if restored {
            log::info!("restored the previous version at {}", target.display());
            self.release_mutex();
            match spawn_instance(&target) {
                Ok(_child) => {
                    self.handed_off.store(true, Ordering::SeqCst);
                }
                Err(error) => {
                    log::error!("could not restart the previous version: {error}");
                    let _ = self.reacquire_mutex(Duration::from_secs(1));
                }
            }
            "the new version did not start correctly; the previous version was restored"
        } else {
            log::error!("the previous version could not be restored");
            "the new version did not start correctly and the previous version could not be restored; download the release from GitHub"
        };
        let failure = UpdateError::new(UpdateStage::Health, reason);
        self.record_update_error(&failure, manual, UpdateStage::Health, Some(reason));
        if self.handed_off.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(500));
            self.stop.store(true, Ordering::SeqCst);
            self.show_quit();
        }
        true
    }
}

fn poll_loop(runtime: Runtime, commands: Receiver<PollCommand>) {
    let mut notifications = Notifications::default();
    let mut signature = protocol::present_product_ids(&*runtime.hid);

    loop {
        if runtime.stop.load(Ordering::SeqCst) {
            return;
        }
        let reading = protocol::read_battery(&*runtime.hid, crate::HID_TIMEOUT);
        {
            let mut state = runtime
                .shared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match reading {
                Some(reading) => {
                    if state.last.as_ref() != Some(&reading) {
                        log::info!(
                            "battery: {}% ({}), {}, {}",
                            reading.percent,
                            core::state_name(reading.state),
                            core::display_name(Some(&reading)),
                            reading.protocol
                        );
                    }
                    state.last = Some(reading);
                    state.last_ok = Some(SystemTime::now());
                    state.online = true;
                }
                None => {
                    if state.online {
                        log::info!("mouse did not respond (asleep or out of range)");
                    }
                    state.online = false;
                }
            }
        }
        runtime.maybe_notify(&mut notifications);
        runtime.write_status(true);
        runtime.send_snapshot();

        let (last, online, _) = runtime.shared_snapshot();
        let interval = core::poll_interval(last.as_ref(), online);
        let deadline = Instant::now() + interval;
        let mut watch_at = Instant::now() + crate::DEVICE_WATCH_INTERVAL;
        while Instant::now() < deadline {
            match commands.recv_timeout(Duration::from_millis(100)) {
                Ok(PollCommand::PollNow) => break,
                Ok(PollCommand::RefreshSnapshot) => runtime.send_snapshot(),
                Ok(PollCommand::Stop) | Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => {}
            }
            if runtime.stop.load(Ordering::SeqCst) {
                return;
            }
            if Instant::now() >= watch_at {
                watch_at += crate::DEVICE_WATCH_INTERVAL;
                let current = protocol::present_product_ids(&*runtime.hid);
                if current != signature {
                    log::info!("device change: {signature:?} -> {current:?}");
                    signature = current;
                    break;
                }
            }
        }
    }
}

fn health_loop(runtime: Runtime) {
    if !sleep_until_stopped(&runtime, crate::HEALTH_DELAY) {
        return;
    }
    runtime.healthy.store(true, Ordering::SeqCst);
    runtime.write_status(true);
    let Some(exe) = runtime.exe.clone() else {
        return;
    };
    if !sleep_until_stopped(&runtime, crate::BACKUP_GRACE_PERIOD) {
        return;
    }
    updater::cleanup_backup(&exe);
    log::info!(
        "removed the rollback copy after {} minutes of uptime",
        crate::BACKUP_GRACE_PERIOD.as_secs() / 60
    );
}

fn sleep_until_stopped(runtime: &Runtime, total: Duration) -> bool {
    let deadline = Instant::now() + total;
    while Instant::now() < deadline {
        if runtime.stop.load(Ordering::SeqCst) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    !runtime.stop.load(Ordering::SeqCst)
}

fn update_loop(runtime: Runtime, commands: Receiver<UpdateCommand>) {
    if !sleep_until_stopped(&runtime, crate::UPDATE_FIRST_CHECK_DELAY) {
        return;
    }
    let mut failures = 0_u32;
    loop {
        if runtime.stop.load(Ordering::SeqCst) {
            return;
        }
        let ok = runtime.check_updates(false);
        let (new_failures, delay) = core::next_update_delay(ok, failures);
        failures = new_failures;
        runtime.write_status(true);
        let deadline = Instant::now() + delay;
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match commands.recv_timeout(remaining.min(Duration::from_secs(1))) {
                Ok(UpdateCommand::CheckNow) => {
                    let _ = runtime.check_updates(true);
                }
                Ok(UpdateCommand::Stop) | Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => {}
            }
            if runtime.stop.load(Ordering::SeqCst) {
                return;
            }
        }
    }
}

fn action_loop(runtime: Runtime, actions: Receiver<MenuAction>) {
    loop {
        match actions.recv_timeout(Duration::from_millis(200)) {
            Ok(action) => {
                if runtime.handle_action(action) {
                    return;
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if runtime.stop.load(Ordering::SeqCst) {
                    return;
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn auto_promote(runtime: Runtime) {
    let Some(exe) = runtime.exe.clone() else {
        return;
    };
    let registry = WindowsRegistry;
    let result = tray_visibility::promote_if_unconfigured(
        &registry,
        &exe,
        win::build_number(),
        tray_visibility::AUTO_DELAYS,
        &|| runtime.stop.load(Ordering::Relaxed),
    );
    log::info!("tray promotion (auto): {result:?}");
}

/// Run the application until the tray is closed. Returns the exit code.
#[must_use]
pub fn run(paths: Paths, mutex: HeldMutex) -> i32 {
    let settings = SettingsStore::new(paths.settings.clone());
    let status = StatusStore::new(paths.status.clone());
    let language = settings.language().unwrap_or_else(Language::detect);
    let update_mode = settings.update_mode();
    let exe = updater::executable_path();

    let hid: Arc<dyn HidPort> = match NativeHid::new() {
        Ok(hid) => Arc::new(hid),
        Err(error) => {
            log::warn!("HID is not available: {error}");
            Arc::new(NoHid)
        }
    };
    let notifier: Arc<dyn Notifier> = Arc::new(ToastNotifier::new());
    if let Some(exe) = &exe {
        ToastNotifier::register_app_id(exe);
    }

    let (ui_tx, ui_rx) = mpsc::channel();
    let (action_tx, action_rx) = mpsc::channel();
    let (poll_tx, poll_rx) = mpsc::channel();
    let (update_tx, update_rx) = mpsc::channel();

    let runtime = Runtime {
        paths,
        settings,
        status,
        hid,
        notifier,
        transport: UreqTransport,
        policy: UrlPolicy::github(),
        language: Arc::new(Mutex::new(language)),
        shared: Arc::new(Mutex::new(SharedState::default())),
        updates: Arc::new(Mutex::new(UpdateState::default())),
        update_mode: Arc::new(Mutex::new(update_mode)),
        stop: Arc::new(AtomicBool::new(false)),
        handed_off: Arc::new(AtomicBool::new(false)),
        update_pending: Arc::new(AtomicBool::new(false)),
        tray_ready: Arc::new(AtomicBool::new(false)),
        healthy: Arc::new(AtomicBool::new(false)),
        ui: ui_tx,
        ui_thread: Arc::new(AtomicU32::new(0)),
        poll: poll_tx,
        update: update_tx,
        mutex: Arc::new(Mutex::new(Some(mutex))),
        exe,
    };

    runtime.write_status(true);

    let poll = start("battery-poller", {
        let runtime = runtime.clone();
        move || poll_loop(runtime, poll_rx)
    });
    let health = start("health-check", {
        let runtime = runtime.clone();
        move || health_loop(runtime)
    });
    let updates = start("update-checker", {
        let runtime = runtime.clone();
        move || update_loop(runtime, update_rx)
    });
    let actions = start("menu-actions", {
        let runtime = runtime.clone();
        move || action_loop(runtime, action_rx)
    });
    if crate::is_packaged()
        && let Some(handle) = start("tray-promoter", {
            let runtime = runtime.clone();
            move || auto_promote(runtime)
        })
    {
        let _ = handle;
    }

    let initial = runtime.build_snapshot();
    let result = tray::run(
        ui_rx,
        action_tx,
        initial,
        &runtime.ui_thread,
        &runtime.tray_ready,
    );

    runtime.stop.store(true, Ordering::SeqCst);
    let _ = runtime.poll.send(PollCommand::Stop);
    let _ = runtime.update.send(UpdateCommand::Stop);
    runtime.wake();
    if let Some(handle) = poll {
        let _ = handle.join();
    }
    if let Some(handle) = health {
        let _ = handle.join();
    }
    if let Some(handle) = updates {
        let _ = handle.join();
    }
    if let Some(handle) = actions {
        let _ = handle.join();
    }
    runtime.write_status(false);

    match result {
        Ok(()) => {
            log::info!("tray stopped");
            0
        }
        Err(error) => {
            log::error!("tray failed: {error}");
            win::message_box(crate::APP_NAME, &error, true);
            1
        }
    }
}
