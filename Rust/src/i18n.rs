//! Russian and English UI strings.
//!
//! Every user-visible text lives in one of two tables that must cover the same
//! [`Key`] enum, so a missing translation is a compile error. Placeholders use
//! the same `{name}` syntax as the Python implementation and are rendered by
//! [`render`] without ever panicking.

use std::fmt;

use crate::win;

/// Languages supported by the user interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Language {
    /// Russian.
    Ru,
    /// English.
    En,
}

impl Language {
    /// All supported languages in menu order.
    pub const ALL: [Self; 2] = [Self::Ru, Self::En];
    /// Fallback language for unknown codes.
    pub const DEFAULT: Self = Self::En;

    /// Parse a stored language code, falling back to English.
    #[must_use]
    pub fn from_code(code: &str) -> Self {
        match code.trim().to_ascii_lowercase().as_str() {
            "ru" => Self::Ru,
            _ => Self::En,
        }
    }

    /// Stable two-letter code used in `settings.json`.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Ru => "ru",
            Self::En => "en",
        }
    }

    /// Language name shown in the tray menu.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Ru => "Русский",
            Self::En => "English",
        }
    }

    /// Detect the language of the current Windows user.
    #[must_use]
    pub fn detect() -> Self {
        if win::system_language_is_russian() {
            Self::Ru
        } else {
            Self::En
        }
    }
}

/// Generate the string key enum and its stable wire names.
macro_rules! keys {
    ($($variant:ident => $name:literal),+ $(,)?) => {
        /// A translatable user-interface string.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[expect(missing_docs, reason = "variant names are the documentation: each is a string key")]
        pub enum Key {
            $($variant),+
        }

        impl Key {
            /// Every key, used by tests to check both tables.
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            /// Stable dotted name, matching the Python implementation.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $name),+
                }
            }
        }
    };
}

keys! {
    MenuRefresh => "menu.refresh",
    MenuAutostart => "menu.autostart",
    MenuLogs => "menu.logs",
    MenuShowIcon => "menu.show_icon",
    MenuReleasePage => "menu.release_page",
    MenuAutoUpdate => "menu.auto_update",
    MenuCheckUpdates => "menu.check_updates",
    MenuInstallUpdate => "menu.install_update",
    MenuLanguage => "menu.language",
    MenuExit => "menu.exit",
    StatusNoMouse => "status.no_mouse",
    StatusOffline => "status.offline",
    TooltipNoConnection => "tooltip.no_connection",
    TooltipOffline => "tooltip.offline",
    NotifyFullTitle => "notify.full_title",
    NotifyFullBody => "notify.full_body",
    NotifyLowTitle => "notify.low_title",
    NotifyLowBody => "notify.low_body",
    TrayTitle => "tray.title",
    TrayPinnedBody => "tray.pinned_body",
    TrayManualBody => "tray.manual_body",
    TraySourceMode => "tray.source_mode",
    CrashTitle => "crash.title",
    CrashBody => "crash.body",
    AgeJustNow => "age.just_now",
    AgeMinutes => "age.minutes",
    AgeHours => "age.hours",
    AgeHoursMinutes => "age.hours_minutes",
    AgeDays => "age.days",
    AgeDaysHours => "age.days_hours",
    StateDischarging => "state.discharging",
    StateCharging => "state.charging",
    StateFull => "state.full",
    StateUnknown => "state.unknown",
    UpdateTitle => "update.title",
    UpdateChecking => "update.checking",
    UpdateBusy => "update.busy",
    UpdateLatest => "update.latest",
    UpdateAvailable => "update.available",
    UpdateInstalling => "update.installing",
    UpdateFailed => "update.failed",
    UpdateFailedReason => "update.failed_reason",
    UpdateDownloading => "update.downloading",
    UpdateDownloadingUnknown => "update.downloading_unknown",
    UpdateSourceMode => "update.source_mode",
    CliDebugHelp => "cli.debug_help",
    CliLangHelp => "cli.lang_help",
    CliRawHelp => "cli.raw_help",
    CliVersionHelp => "cli.version_help",
    DiagnoseVersion => "diagnose.version",
    DiagnoseExecutable => "diagnose.executable",
    DiagnoseHeader => "diagnose.header",
    DiagnoseHidError => "diagnose.hid_error",
    DiagnoseNotFound => "diagnose.not_found",
    DiagnoseControl => "diagnose.control",
    DiagnoseModelUnknown => "diagnose.model_unknown",
    DiagnoseRawHeader => "diagnose.raw_header",
    DiagnoseRawNone => "diagnose.raw_none",
    DiagnoseNoResponse => "diagnose.no_response",
    DiagnoseBattery => "diagnose.battery",
    DiagnoseRawLevel => "diagnose.raw_level",
    DiagnoseVoltage => "diagnose.voltage",
    DiagnoseShown => "diagnose.shown",
}

const fn english(key: Key) -> &'static str {
    match key {
        Key::MenuRefresh => "Refresh battery manually",
        Key::MenuAutostart => "Start with Windows",
        Key::MenuLogs => "Open data folder",
        Key::MenuShowIcon => "Show icon on taskbar",
        Key::MenuReleasePage => "Open release page",
        Key::MenuAutoUpdate => "Install updates automatically",
        Key::MenuCheckUpdates => "Check for updates",
        Key::MenuInstallUpdate => "Install {version}",
        Key::MenuLanguage => "Language",
        Key::MenuExit => "Exit",
        Key::StatusNoMouse => "No mouse connection",
        Key::StatusOffline => "{percent}% · no connection ({age})",
        Key::TooltipNoConnection => "{name}: no connection — move the mouse",
        Key::TooltipOffline => "{name}: {percent}% (no connection, {age} ago)",
        Key::NotifyFullTitle => "Mouse charged",
        Key::NotifyFullBody => "{name}: {percent}%",
        Key::NotifyLowTitle => "Low battery",
        Key::NotifyLowBody => "{name}: {percent}% — time to charge the mouse",
        Key::TrayTitle => "Tray icon",
        Key::TrayPinnedBody => "The icon is pinned to the taskbar.",
        Key::TrayManualBody => "If the icon is still hidden, enable it in Settings → Taskbar.",
        Key::TraySourceMode => "This works only in the built LamzuBattery.exe.",
        Key::CrashTitle => "LAMZU Battery",
        Key::CrashBody => "LAMZU Battery crashed. See data\\app.log and data\\crash.log.",
        Key::AgeJustNow => "just now",
        Key::AgeMinutes => "{minutes} min",
        Key::AgeHours => "{hours} h",
        Key::AgeHoursMinutes => "{hours} h {minutes} min",
        Key::AgeDays => "{days} d",
        Key::AgeDaysHours => "{days} d {hours} h",
        Key::StateDischarging => "discharging",
        Key::StateCharging => "charging",
        Key::StateFull => "full",
        Key::StateUnknown => "state {state}",
        Key::UpdateTitle => "Update",
        Key::UpdateChecking => "Checking for updates...",
        Key::UpdateBusy => "An update check is already running.",
        Key::UpdateLatest => "You have the latest version ({version}).",
        Key::UpdateAvailable => {
            "Version {version} is available. Use Install {version} in the tray menu."
        }
        Key::UpdateInstalling => "Version {version} installed. Restarting...",
        Key::UpdateFailed => "Could not update. Download the new version from GitHub.",
        Key::UpdateFailedReason => {
            "Could not update: {reason}. Open the release page from the tray menu."
        }
        Key::UpdateDownloading => "Downloading update... {percent}%",
        Key::UpdateDownloadingUnknown => "Downloading update...",
        Key::UpdateSourceMode => "Auto-update works only in the built LamzuBattery.exe.",
        Key::CliDebugHelp => "verbose log",
        Key::CliLangHelp => "interface language (default: system)",
        Key::CliRawHelp => "also print the raw HID reports",
        Key::CliVersionHelp => "print the version and exit",
        Key::DiagnoseVersion => "Version: {version}",
        Key::DiagnoseExecutable => "Executable: {path}",
        Key::DiagnoseHeader => "LAMZU HID interfaces:",
        Key::DiagnoseHidError => "HID enumeration failed: {error}",
        Key::DiagnoseNotFound => "  not found — check that the dongle or cable is connected",
        Key::DiagnoseControl => "  <-- control: {protocol}, {model}",
        Key::DiagnoseModelUnknown => "model not detected",
        Key::DiagnoseRawHeader => "Raw reports:",
        Key::DiagnoseRawNone => "  no report received",
        Key::DiagnoseNoResponse => "Battery: no response. Move the mouse and try again.",
        Key::DiagnoseBattery => "Battery: {percent}% ({state}), {model}, {protocol}",
        Key::DiagnoseRawLevel => "level byte: {percent}%",
        Key::DiagnoseVoltage => "voltage: {voltage} mV -> {percent}%",
        Key::DiagnoseShown => "shown: {percent}%",
    }
}

const fn russian(key: Key) -> &'static str {
    match key {
        Key::MenuRefresh => "Обновить заряд вручную",
        Key::MenuAutostart => "Запускать при входе",
        Key::MenuLogs => "Открыть папку данных",
        Key::MenuShowIcon => "Показать иконку на панели задач",
        Key::MenuReleasePage => "Открыть страницу релиза",
        Key::MenuAutoUpdate => "Устанавливать обновления автоматически",
        Key::MenuCheckUpdates => "Проверить обновления",
        Key::MenuInstallUpdate => "Установить {version}",
        Key::MenuLanguage => "Язык",
        Key::MenuExit => "Выход",
        Key::StatusNoMouse => "Нет связи с мышью",
        Key::StatusOffline => "{percent}% · нет связи ({age})",
        Key::TooltipNoConnection => "{name}: нет связи — пошевелите мышью",
        Key::TooltipOffline => "{name}: {percent}% (нет связи, {age} назад)",
        Key::NotifyFullTitle => "Мышь заряжена",
        Key::NotifyFullBody => "{name}: {percent}%",
        Key::NotifyLowTitle => "Низкий заряд",
        Key::NotifyLowBody => "{name}: {percent}% — пора зарядить мышь",
        Key::TrayTitle => "Иконка в трее",
        Key::TrayPinnedBody => "Иконка закреплена на панели задач.",
        Key::TrayManualBody => {
            "Если иконка всё ещё скрыта, включите её в Параметры → Панель задач."
        }
        Key::TraySourceMode => "Работает только в собранном LamzuBattery.exe.",
        Key::CrashTitle => "LAMZU Battery",
        Key::CrashBody => {
            "Приложение LAMZU Battery аварийно завершилось. \
Подробности в data\\app.log и data\\crash.log."
        }
        Key::AgeJustNow => "только что",
        Key::AgeMinutes => "{minutes} мин",
        Key::AgeHours => "{hours} ч",
        Key::AgeHoursMinutes => "{hours} ч {minutes} мин",
        Key::AgeDays => "{days} д",
        Key::AgeDaysHours => "{days} д {hours} ч",
        Key::StateDischarging => "разряд",
        Key::StateCharging => "заряжается",
        Key::StateFull => "заряжена",
        Key::StateUnknown => "состояние {state}",
        Key::UpdateTitle => "Обновление",
        Key::UpdateChecking => "Проверяю обновления...",
        Key::UpdateBusy => "Проверка обновлений уже выполняется.",
        Key::UpdateLatest => "Установлена последняя версия ({version}).",
        Key::UpdateAvailable => {
            "Доступна версия {version}. Используйте «Установить {version}» в меню."
        }
        Key::UpdateInstalling => "Версия {version} установлена. Перезапуск...",
        Key::UpdateFailed => "Не удалось обновить. Скачайте новую версию с GitHub.",
        Key::UpdateFailedReason => {
            "Не удалось обновить: {reason}. Откройте страницу релиза в меню."
        }
        Key::UpdateDownloading => "Скачиваю обновление... {percent}%",
        Key::UpdateDownloadingUnknown => "Скачиваю обновление...",
        Key::UpdateSourceMode => "Автообновление работает только в собранном LamzuBattery.exe.",
        Key::CliDebugHelp => "подробный журнал",
        Key::CliLangHelp => "язык интерфейса (по умолчанию — системный)",
        Key::CliRawHelp => "дополнительно печатать сырые HID-отчёты",
        Key::CliVersionHelp => "показать версию и выйти",
        Key::DiagnoseVersion => "Версия: {version}",
        Key::DiagnoseExecutable => "Файл: {path}",
        Key::DiagnoseHeader => "LAMZU HID-интерфейсы:",
        Key::DiagnoseHidError => "Не удалось перечислить HID-устройства: {error}",
        Key::DiagnoseNotFound => "  не найдено — проверьте, что донгл или кабель подключён",
        Key::DiagnoseControl => "  <-- управляющий: {protocol}, {model}",
        Key::DiagnoseModelUnknown => "модель не определена",
        Key::DiagnoseRawHeader => "Сырые отчёты:",
        Key::DiagnoseRawNone => "  отчётов нет",
        Key::DiagnoseNoResponse => "Заряд: нет ответа. Пошевелите мышь и запустите ещё раз.",
        Key::DiagnoseBattery => "Заряд: {percent}% ({state}), {model}, {protocol}",
        Key::DiagnoseRawLevel => "байт уровня: {percent} %",
        Key::DiagnoseVoltage => "напряжение: {voltage} мВ -> {percent} %",
        Key::DiagnoseShown => "показано: {percent} %",
    }
}

/// Raw template for a key in a specific language.
#[must_use]
pub const fn template(language: Language, key: Key) -> &'static str {
    match language {
        Language::Ru => russian(key),
        Language::En => english(key),
    }
}

/// Render `{name}` placeholders in a template.
///
/// Unknown placeholders are left untouched and malformed braces are copied
/// verbatim; the function never panics.
#[must_use]
pub fn render(template: &str, arguments: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else {
            out.push('{');
            out.push_str(after);
            return out;
        };
        let name = &after[..end];
        match arguments.iter().find(|(key, _)| *key == name) {
            Some((_, value)) => out.push_str(value),
            None => {
                out.push('{');
                out.push_str(name);
                out.push('}');
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Translate a key without arguments.
#[must_use]
pub fn tr(language: Language, key: Key) -> &'static str {
    template(language, key)
}

/// Translate a key and substitute placeholders.
#[must_use]
pub fn tr_args(language: Language, key: Key, arguments: &[(&str, &str)]) -> String {
    render(template(language, key), arguments)
}

/// Localized battery state, matching the reading semantics.
#[must_use]
pub fn state_text(language: Language, state: crate::protocol::ChargeState) -> String {
    use crate::protocol::ChargeState;
    match state {
        ChargeState::Full => tr(language, Key::StateFull).to_owned(),
        ChargeState::Discharging => tr(language, Key::StateDischarging).to_owned(),
        ChargeState::Charging => tr(language, Key::StateCharging).to_owned(),
        ChargeState::Other(raw) => {
            tr_args(language, Key::StateUnknown, &[("state", &raw.to_string())])
        }
    }
}

/// Short localized duration such as `5 min` or `2 h 10 min`.
#[must_use]
pub fn human_age(language: Language, seconds: u64) -> String {
    if seconds < 60 {
        return tr(language, Key::AgeJustNow).to_owned();
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return tr_args(
            language,
            Key::AgeMinutes,
            &[("minutes", &minutes.to_string())],
        );
    }
    let hours = minutes / 60;
    let minutes = minutes % 60;
    if hours < 24 {
        if minutes > 0 {
            return tr_args(
                language,
                Key::AgeHoursMinutes,
                &[
                    ("hours", &hours.to_string()),
                    ("minutes", &minutes.to_string()),
                ],
            );
        }
        return tr_args(language, Key::AgeHours, &[("hours", &hours.to_string())]);
    }
    let days = hours / 24;
    let hours = hours % 24;
    if hours > 0 {
        return tr_args(
            language,
            Key::AgeDaysHours,
            &[("days", &days.to_string()), ("hours", &hours.to_string())],
        );
    }
    tr_args(language, Key::AgeDays, &[("days", &days.to_string())])
}

impl fmt::Display for Language {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}
