"""Russian/English UI strings for LAMZU Battery.

The tray, notifications and desktop overlay all use this module. The selected
language is stored by the application in ``data/settings.json``; the overlay
receives the current language through ``status.json`` so it can update without
a restart.
"""

from __future__ import annotations

import ctypes
from typing import Optional

LANGUAGES = ("ru", "en")
DEFAULT_LANGUAGE = "en"
LANGUAGE_NAMES = {"ru": "Русский", "en": "English"}

_STRINGS = {
    "en": {
        "menu.refresh": "Refresh now",
        "menu.desktop": "Show on desktop",
        "menu.autostart": "Start with Windows",
        "menu.logs": "Open logs",
        "menu.check_updates": "Check for updates",
        "menu.language": "Language",
        "menu.exit": "Exit",
        "status.no_mouse": "No mouse connection",
        "status.offline": "{percent}% · no connection ({age})",
        "tooltip.no_connection": "{name}: no connection — move the mouse",
        "tooltip.offline": "{name}: {percent}% (no connection, {age} ago)",
        "notify.full_title": "Mouse charged",
        "notify.full_body": "{name}: {percent}%",
        "notify.low_title": "Low battery",
        "notify.low_body": "{name}: {percent}% — time to charge the mouse",
        "age.just_now": "just now",
        "age.minutes": "{minutes} min",
        "age.hours": "{hours} h",
        "age.hours_minutes": "{hours} h {minutes} min",
        "age.days": "{days} d",
        "age.days_hours": "{days} d {hours} h",
        "state.discharging": "discharging",
        "state.charging": "charging",
        "state.full": "full",
        "state.unknown": "state {state}",
        "overlay.no_data": "no data",
        "overlay.stale": "data is stale",
        "overlay.no_connection": "no connection",
        "overlay.close": "Close",
        "update.title": "Update",
        "update.checking": "Checking for updates...",
        "update.busy": "An update check is already running.",
        "update.latest": "You have the latest version ({version}).",
        "update.installing": "Version {version} installed. Restarting...",
        "update.failed": "Could not update. Download the new version from GitHub.",
        "update.source_mode": "Auto-update works only in the built LamzuBattery.exe.",
        "cli.overlay_help": "desktop overlay window",
        "cli.debug_help": "verbose log",
        "cli.lang_help": "interface language (default: system)",
        "diagnose.version": "Version: {version}",
        "diagnose.executable": "Executable: {path}",
        "diagnose.header": "LAMZU HID interfaces:",
        "diagnose.not_found": (
            "  not found — check that the dongle or cable is connected"
        ),
        "diagnose.control": "  <-- control: {protocol}, {model}",
        "diagnose.model_unknown": "model not detected",
        "diagnose.no_response": (
            "Battery: no response. Move the mouse and try again."
        ),
        "diagnose.battery": "Battery: {percent}% ({state}), {model}, {protocol}",
        "diagnose.raw_level": "level byte: {percent}%",
        "diagnose.voltage": "voltage: {voltage} mV -> {percent}%",
        "diagnose.shown": "shown: {percent}%",
    },
    "ru": {
        "menu.refresh": "Обновить сейчас",
        "menu.desktop": "Показывать на рабочем столе",
        "menu.autostart": "Запускать при входе",
        "menu.logs": "Открыть журнал",
        "menu.check_updates": "Проверить обновления",
        "menu.language": "Язык",
        "menu.exit": "Выход",
        "status.no_mouse": "Нет связи с мышью",
        "status.offline": "{percent}% · нет связи ({age})",
        "tooltip.no_connection": "{name}: нет связи — пошевелите мышью",
        "tooltip.offline": "{name}: {percent}% (нет связи, {age} назад)",
        "notify.full_title": "Мышь заряжена",
        "notify.full_body": "{name}: {percent}%",
        "notify.low_title": "Низкий заряд",
        "notify.low_body": "{name}: {percent}% — пора зарядить мышь",
        "age.just_now": "только что",
        "age.minutes": "{minutes} мин",
        "age.hours": "{hours} ч",
        "age.hours_minutes": "{hours} ч {minutes} мин",
        "age.days": "{days} д",
        "age.days_hours": "{days} д {hours} ч",
        "state.discharging": "разряд",
        "state.charging": "заряжается",
        "state.full": "заряжена",
        "state.unknown": "состояние {state}",
        "overlay.no_data": "нет данных",
        "overlay.stale": "данные устарели",
        "overlay.no_connection": "нет связи",
        "overlay.close": "Закрыть",
        "update.title": "Обновление",
        "update.checking": "Проверяю обновления...",
        "update.busy": "Проверка обновлений уже выполняется.",
        "update.latest": "Установлена последняя версия ({version}).",
        "update.installing": "Версия {version} установлена. Перезапуск...",
        "update.failed": "Не удалось обновить. Скачайте новую версию с GitHub.",
        "update.source_mode": "Автообновление работает только в собранном LamzuBattery.exe.",
        "cli.overlay_help": "окно на рабочем столе",
        "cli.debug_help": "подробный журнал",
        "cli.lang_help": "язык интерфейса (по умолчанию — системный)",
        "diagnose.version": "Версия: {version}",
        "diagnose.executable": "Файл: {path}",
        "diagnose.header": "LAMZU HID-интерфейсы:",
        "diagnose.not_found": (
            "  не найдено — проверьте, что донгл или кабель подключён"
        ),
        "diagnose.control": "  <-- управляющий: {protocol}, {model}",
        "diagnose.model_unknown": "модель не определена",
        "diagnose.no_response": (
            "Заряд: нет ответа. Пошевелите мышь и запустите ещё раз."
        ),
        "diagnose.battery": "Заряд: {percent}% ({state}), {model}, {protocol}",
        "diagnose.raw_level": "байт уровня: {percent} %",
        "diagnose.voltage": "напряжение: {voltage} мВ -> {percent} %",
        "diagnose.shown": "показано: {percent} %",
    },
}

_current = DEFAULT_LANGUAGE


def normalize_language(language: Optional[str]) -> str:
    """Return a supported language code, falling back to English."""
    return language if language in LANGUAGES else DEFAULT_LANGUAGE


def set_language(language: Optional[str]) -> str:
    """Select the UI language for this process and return the normalized code."""
    global _current
    _current = normalize_language(language)
    return _current


def get_language() -> str:
    return _current


def detect_system_language() -> str:
    """Best-effort Windows UI language detection (Russian or English)."""
    try:
        lang_id = ctypes.windll.kernel32.GetUserDefaultUILanguage()
        if (int(lang_id) & 0x3FF) == 0x19:  # LANG_RUSSIAN
            return "ru"
    except Exception:
        pass
    return "en"


def translate(key: str, language: Optional[str] = None, **values) -> str:
    """Translate a key to the given (or current) language."""
    table = _STRINGS[normalize_language(language or _current)]
    text = table.get(key) or _STRINGS[DEFAULT_LANGUAGE].get(key) or key
    if not values:
        return text
    try:
        return text.format(**values)
    except (KeyError, IndexError, ValueError):
        return text


def tr(key: str, **values) -> str:
    """Translate a key to the current language."""
    return translate(key, _current, **values)


def state_text(state: Optional[int], percent: Optional[int] = None,
               language: Optional[str] = None) -> str:
    """Localized battery state, matching BatteryReading semantics."""
    if state == 2 or (state == 1 and (percent or 0) >= 100):
        key = "state.full"
    elif state == 0:
        key = "state.discharging"
    elif state == 1:
        key = "state.charging"
    else:
        return translate("state.unknown", language, state=state)
    return translate(key, language)


def human_age(seconds: float, language: Optional[str] = None) -> str:
    """Short localized duration, e.g. «5 мин» or «2 h 10 min»."""
    seconds = max(0, int(seconds))
    if seconds < 60:
        return translate("age.just_now", language)
    minutes = seconds // 60
    if minutes < 60:
        return translate("age.minutes", language, minutes=minutes)
    hours, minutes = divmod(minutes, 60)
    if hours < 24:
        if minutes:
            return translate(
                "age.hours_minutes", language, hours=hours, minutes=minutes
            )
        return translate("age.hours", language, hours=hours)
    days, hours = divmod(hours, 24)
    if hours:
        return translate("age.days_hours", language, days=days, hours=hours)
    return translate("age.days", language, days=days)
