"""Promote the tray icon on Windows 11 without fighting the user.

Windows 11 22H2+ stores per-icon visibility in
``HKCU\\Control Panel\\NotifyIconSettings``.  A missing ``IsPromoted`` value
means the icon was never configured and is usually hidden in the overflow
flyout.  We set it to ``1`` only in that case.  An explicit ``0`` means the
user hid the icon, so it is left alone.

The registry key is undocumented, so every operation is best-effort and the
caller always has a manual fallback in the tray menu.
"""

from __future__ import annotations

import logging
import os
import sys
import time
import winreg
from dataclasses import dataclass
from typing import Optional

log = logging.getLogger("lamzu_tray.tray_visibility")

NOTIFY_ICON_KEY = r"Control Panel\NotifyIconSettings"
EXECUTABLE_PATH = "ExecutablePath"
IS_PROMOTED = "IsPromoted"
MIN_BUILD = 22621

AUTO_RETRY_DELAYS = (0.5, 1.0, 2.0, 3.0, 5.0, 8.0, 10.0, 10.0, 10.0, 10.0)
MANUAL_RETRY_DELAYS = (0.5, 1.0, 2.0, 5.0)

PROMOTED = "promoted"
ALREADY = "already"
OPTED_OUT = "opted_out"
UNSUPPORTED = "unsupported"
NOT_FOUND = "not_found"
ERROR = "error"
STOPPED = "stopped"


@dataclass(frozen=True)
class TrayEntry:
    key_name: str
    promoted: Optional[int]


def is_supported() -> bool:
    """The undocumented key is used by Windows 11 22H2 and newer."""
    if sys.platform != "win32":
        return False
    try:
        return sys.getwindowsversion().build >= MIN_BUILD
    except AttributeError:
        return False


def _normalize(path: object) -> str:
    value = str(path or "").strip().strip('"')
    value = os.path.expandvars(value)
    return os.path.normcase(os.path.normpath(value))


def _entries_for(exe_path: object) -> list[TrayEntry]:
    """Return NotifyIconSettings entries whose exe path matches."""
    target = _normalize(exe_path)
    if not target:
        return []
    entries: list[TrayEntry] = []
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, NOTIFY_ICON_KEY) as root:
            for index in range(winreg.QueryInfoKey(root)[0]):
                key_name = winreg.EnumKey(root, index)
                try:
                    with winreg.OpenKey(root, key_name) as key:
                        executable, _ = winreg.QueryValueEx(
                            key, EXECUTABLE_PATH
                        )
                        if _normalize(executable) != target:
                            continue
                        try:
                            promoted, _ = winreg.QueryValueEx(
                                key, IS_PROMOTED
                            )
                            promoted_value: Optional[int] = int(promoted)
                        except (FileNotFoundError, TypeError, ValueError):
                            promoted_value = None
                        entries.append(TrayEntry(key_name, promoted_value))
                except OSError:
                    continue
    except OSError:
        return []
    return entries


def _set_promoted(key_name: str, value: int = 1) -> None:
    path = f"{NOTIFY_ICON_KEY}\\{key_name}"
    with winreg.OpenKey(
        winreg.HKEY_CURRENT_USER, path, 0, winreg.KEY_SET_VALUE
    ) as key:
        winreg.SetValueEx(key, IS_PROMOTED, 0, winreg.REG_DWORD, value)


def _wait(delay: float, stop_event=None) -> bool:
    if delay <= 0:
        return bool(stop_event is not None and stop_event.is_set())
    if stop_event is not None:
        return stop_event.wait(delay)
    time.sleep(delay)
    return False


def promote_if_unconfigured(
    exe_path: object,
    stop_event=None,
    retry_delays: tuple[float, ...] = AUTO_RETRY_DELAYS,
) -> str:
    """Promote only a never-configured icon; respect an explicit 0."""
    if not is_supported():
        return UNSUPPORTED

    for delay in (0.0, *retry_delays):
        if _wait(delay, stop_event):
            return STOPPED
        entries = _entries_for(exe_path)
        if not entries:
            continue
        if any(entry.promoted == 0 for entry in entries):
            return OPTED_OUT
        missing = [entry for entry in entries if entry.promoted is None]
        if not missing:
            return ALREADY
        try:
            _set_promoted(missing[0].key_name)
        except OSError:
            log.debug("could not promote tray icon", exc_info=True)
            return ERROR
        return PROMOTED
    return NOT_FOUND


def promote(
    exe_path: object,
    retry_delays: tuple[float, ...] = MANUAL_RETRY_DELAYS,
) -> str:
    """Explicit user request to put the icon on the taskbar."""
    if not is_supported():
        return UNSUPPORTED

    for delay in (0.0, *retry_delays):
        if _wait(delay, None):
            return STOPPED
        entries = _entries_for(exe_path)
        if not entries:
            continue
        try:
            for entry in entries:
                _set_promoted(entry.key_name)
        except OSError:
            log.debug("could not promote tray icon", exc_info=True)
            return ERROR
        return PROMOTED
    return NOT_FOUND


def open_taskbar_settings() -> None:
    if sys.platform != "win32":
        return
    try:
        os.startfile("ms-settings:taskbar")  # noqa: S606
    except OSError:
        log.debug("could not open taskbar settings", exc_info=True)
