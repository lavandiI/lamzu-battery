"""LAMZU Battery — индикатор заряда мышей LAMZU в системном трее Windows.

Поддерживаются модели Lamzu с протоколами Compx и Aurora (см. lamzu.py).
Запуск:  pythonw app.py
"""

from __future__ import annotations

import argparse
import ctypes
import faulthandler
import json
import logging
import os
import subprocess
import sys
import tempfile
import threading
import time
import webbrowser
import winreg
from functools import lru_cache
from logging.handlers import RotatingFileHandler
from pathlib import Path
from typing import Optional

import pystray
from PIL import Image, ImageDraw, ImageFont

import i18n
import tray_visibility
import updater
from lamzu import BatteryReading, present_product_ids, read_battery
from version import APP_VERSION

APP_NAME = "LAMZU Battery"
APP_ID = "LamzuBattery"
MUTEX_TRAY = "Local\\LamzuBatteryTray"
RUN_KEY = r"Software\Microsoft\Windows\CurrentVersion\Run"
RUN_VALUE = "LamzuBattery"
LEGACY_RUN_VALUE = "LamzuThornBattery"
CREATE_NO_WINDOW = 0x08000000
RELEASES_URL = "https://github.com/lavandiI/lamzu-battery/releases/latest"
UPDATE_CHECK_INTERVAL = 6 * 60 * 60
UPDATE_RETRY_DELAYS = (60, 300, 900, 3600, 21600)
HEALTH_DELAY = 5.0
UPDATE_READY_TIMEOUT = 30.0
# How long the rollback copy is kept after a successful start. A defect that
# shows up minutes later can still be rolled back by hand; the copy is also
# removed before the next update in updater.prepare_update_target().
BACKUP_GRACE_PERIOD = 60 * 60

if getattr(sys, "frozen", False):
    # PyInstaller: файлы рядом с exe, а не во временной папке _MEIPASS.
    BASE_DIR = Path(sys.executable).resolve().parent
else:
    BASE_DIR = Path(__file__).resolve().parent


_DATA_DIR_FALLBACK: Optional[Path] = None


def _is_writable_dir(path: Path) -> bool:
    """Create the folder and prove it is writable with a unique probe file."""
    probe = path / f".write_test-{os.getpid()}"
    try:
        path.mkdir(parents=True, exist_ok=True)
        probe.write_text("ok", encoding="ascii")
    except OSError:
        return False
    try:
        # A unique name keeps two instances starting at the same time from
        # deleting each other's probe and wrongly falling back.
        probe.unlink(missing_ok=True)
    except OSError:
        pass
    return True


def _pick_data_dir() -> Path:
    """Портативная папка data рядом с приложением, иначе LocalAppData.

    Важно: Python из Microsoft Store виртуализирует запись в AppData,
    поэтому основной вариант — обычная папка рядом с программой.
    """
    global _DATA_DIR_FALLBACK
    preferred = BASE_DIR / "data"
    if _is_writable_dir(preferred):
        return preferred
    local = os.environ.get("LOCALAPPDATA") or Path.home()
    for candidate in (
        Path(local) / APP_ID,
        Path(tempfile.gettempdir()) / APP_ID,
    ):
        if _is_writable_dir(candidate):
            _DATA_DIR_FALLBACK = preferred
            return candidate
    # Nothing is writable: keep the portable path so the failure is visible
    # in one place instead of hiding behind a different directory.
    _DATA_DIR_FALLBACK = preferred
    return preferred


DATA_DIR = _pick_data_dir()
STATUS_FILE = DATA_DIR / "status.json"
SETTINGS_FILE = DATA_DIR / "settings.json"
LOG_FILE = DATA_DIR / "app.log"
CRASH_FILE = DATA_DIR / "crash.log"

DEFAULT_UPDATE_MODE = "auto"
UPDATE_MODES = ("auto", "notify")

# Цифры на тёмной панели задач — яркие, на светлой — заметно темнее:
# светло-голубой «нет данных» на #F3F3F3 даёт контраст 1.36:1, то есть
# практически не виден. Для светлой темы все цвета держат >= 5:1.
PALETTE_DARK = {
    "normal": (0, 122, 255, 255),       # синий
    "low": (255, 59, 48, 255),          # яркий красный
    "charge": (0, 140, 70, 255),        # тёмно-зелёный
    "stale": (184, 212, 255, 255),      # светло-голубой вместо серого
}
PALETTE_LIGHT = {
    "normal": (0, 88, 200, 255),        # 5.86:1 на #F3F3F3
    "low": (196, 39, 30, 255),          # 5.18:1
    "charge": (0, 107, 52, 255),        # 6.00:1
    "stale": (44, 95, 168, 255),        # 5.72:1
}
PERSONALIZE_KEY = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"
LIGHT_THEME_VALUE = "SystemUsesLightTheme"


def palette(light: bool = False) -> dict[str, tuple[int, int, int, int]]:
    """Return the tray palette for the current taskbar theme."""
    return PALETTE_LIGHT if light else PALETTE_DARK


def taskbar_uses_light_theme() -> bool:
    """True when Windows draws the taskbar light.

    Windows 10 has no such value for the taskbar (it is always dark), so a
    missing key means the dark palette, which is also the old behaviour.
    """
    try:
        with winreg.OpenKey(
            winreg.HKEY_CURRENT_USER, PERSONALIZE_KEY
        ) as key:
            value, _ = winreg.QueryValueEx(key, LIGHT_THEME_VALUE)
        return int(value) == 1
    except (OSError, TypeError, ValueError):
        return False


def battery_color(
    percent: int, charging: bool, full: bool, light: bool = False
) -> tuple[int, int, int, int]:
    """Цвет цифр: зелёный при зарядке, красный ниже 30%, иначе синий."""
    colors = palette(light)
    if charging or full:
        return colors["charge"]
    if percent < 30:
        return colors["low"]
    return colors["normal"]


# Обычное обновление — раз в 5 минут. Во время зарядки чаще, чтобы
# не пропустить 100%; после неудачного опроса — каждые 20 секунд, чтобы
# быстрее поймать проснувшуюся мышь.
POLL_INTERVAL = 5 * 60
POLL_INTERVAL_CHARGING = 60
POLL_INTERVAL_OFFLINE = 20

log = logging.getLogger("lamzu_tray")


def _as_int(value) -> int:
    """Best-effort int conversion for a value read from a JSON file."""
    try:
        return int(value)
    except (TypeError, ValueError):
        return 0


def _next_update_delay(ok: bool, failures: int) -> tuple[int, float]:
    """Return (new_failure_count, seconds_to_wait) for the update loop."""
    if ok:
        return 0, UPDATE_CHECK_INTERVAL
    delay = UPDATE_RETRY_DELAYS[
        min(failures, len(UPDATE_RETRY_DELAYS) - 1)
    ]
    return failures + 1, delay


def _load_settings() -> dict:
    """Read settings, keeping unknown keys for forward compatibility."""
    try:
        data = json.loads(SETTINGS_FILE.read_text(encoding="utf-8-sig"))
    except (OSError, ValueError, TypeError, AttributeError):
        data = None
    if not isinstance(data, dict):
        data = {}
    settings = dict(data)
    mode = str(settings.get("update_mode") or DEFAULT_UPDATE_MODE).lower()
    settings["update_mode"] = (
        mode if mode in UPDATE_MODES else DEFAULT_UPDATE_MODE
    )
    return settings


def _normalize_update_mode(value: object) -> str:
    mode = str(value or DEFAULT_UPDATE_MODE).lower()
    return mode if mode in UPDATE_MODES else DEFAULT_UPDATE_MODE


def _save_settings(**changes: object) -> None:
    """Merge changes into settings.json instead of replacing the file."""
    settings = _load_settings()
    settings.update(changes)
    settings["update_mode"] = _normalize_update_mode(
        settings.get("update_mode")
    )
    tmp: Optional[Path] = None
    try:
        DATA_DIR.mkdir(parents=True, exist_ok=True)
        # A unique name: two instances (autostart plus a manual start, or the
        # update hand-off) must not publish a mixed file.
        tmp = SETTINGS_FILE.with_name(
            f"{SETTINGS_FILE.name}.{os.getpid()}.tmp"
        )
        tmp.write_text(
            json.dumps(settings, ensure_ascii=False, indent=2),
            encoding="utf-8",
        )
        os.replace(tmp, SETTINGS_FILE)
    except OSError:
        log.debug("could not save settings", exc_info=True)
        if tmp is not None:
            try:
                tmp.unlink(missing_ok=True)
            except OSError:
                pass


def _load_language() -> str:
    """Прочитать выбранный язык; при первом запуске — язык системы."""
    language = _load_settings().get("language")
    return i18n.normalize_language(
        language or i18n.detect_system_language()
    )


def _save_language(language: str) -> None:
    _save_settings(language=language)


# ---------------------------------------------------------------------------
# Общие помощники
# ---------------------------------------------------------------------------

def _file_log_handler(path: Path) -> RotatingFileHandler:
    handler = RotatingFileHandler(
        path, maxBytes=256 * 1024, backupCount=1, encoding="utf-8"
    )
    handler.setFormatter(
        logging.Formatter(
            "%(asctime)s %(levelname)s %(message)s", "%Y-%m-%d %H:%M:%S"
        )
    )
    return handler


def _has_log_handler(path: Path) -> bool:
    target = Path(path)
    for handler in logging.getLogger().handlers:
        if isinstance(handler, RotatingFileHandler):
            if Path(handler.baseFilename) == target:
                return True
    return False


def setup_logging(debug: bool = False) -> bool:
    """Configure file logging; this never raises.

    A read-only data folder must not kill a windowed application before the
    crash dialog can be installed, so the log moves to ``%TEMP%`` and, if
    that fails too, logging is simply disabled. Returns True when a log file
    is active.
    """
    root = logging.getLogger()
    root.setLevel(logging.DEBUG if debug else logging.INFO)
    if _has_log_handler(LOG_FILE):
        return True

    handler: Optional[RotatingFileHandler] = None
    try:
        DATA_DIR.mkdir(parents=True, exist_ok=True)
        handler = _file_log_handler(LOG_FILE)
    except OSError:
        try:
            handler = _file_log_handler(
                Path(tempfile.gettempdir()) / f"{APP_ID}.log"
            )
        except OSError:
            handler = None
    if handler is not None:
        root.addHandler(handler)

    # Не засорять журнал внутренними сообщениями библиотек.
    for noisy in ("PIL", "pystray", "hid"):
        logging.getLogger(noisy).setLevel(logging.INFO)
    if debug and sys.stderr is not None:
        console = logging.StreamHandler()
        console.setFormatter(
            logging.Formatter(
                "%(asctime)s %(levelname)s %(message)s",
                "%Y-%m-%d %H:%M:%S",
            )
        )
        root.addHandler(console)

    if handler is None:
        return False
    if Path(handler.baseFilename) != Path(LOG_FILE):
        log.warning(
            "could not write %s, logging to %s", LOG_FILE, handler.baseFilename
        )
    return True


_FAULT_FILE = None


def _show_crash_message() -> None:
    try:
        ctypes.windll.user32.MessageBoxW(
            None,
            i18n.tr("crash.body"),
            i18n.tr("crash.title"),
            0x10,
        )
    except Exception:
        pass


def _unhandled_exception(exc_type, exc_value, exc_traceback) -> None:
    if issubclass(exc_type, SystemExit):
        return
    if issubclass(exc_type, KeyboardInterrupt):
        sys.__excepthook__(exc_type, exc_value, exc_traceback)
        return
    log.critical(
        "unhandled exception",
        exc_info=(exc_type, exc_value, exc_traceback),
    )
    _show_crash_message()


def _unhandled_thread(args) -> None:
    if args.exc_type is SystemExit:
        return
    thread_name = args.thread.name if args.thread is not None else "unknown"
    log.critical(
        "unhandled exception in thread %s",
        thread_name,
        exc_info=(args.exc_type, args.exc_value, args.exc_traceback),
    )


def install_exception_logging() -> None:
    """Log Python and native crashes; a windowed exe has no stderr."""
    global _FAULT_FILE
    if _FAULT_FILE is None:
        try:
            if CRASH_FILE.exists() and CRASH_FILE.stat().st_size > 256 * 1024:
                CRASH_FILE.unlink()
        except OSError:
            pass
        fault_file = None
        try:
            fault_file = open(CRASH_FILE, "a", encoding="utf-8")
            faulthandler.enable(file=fault_file, all_threads=True)
            _FAULT_FILE = fault_file
        except (OSError, RuntimeError, ValueError):
            # faulthandler may refuse the file after it was opened; close it
            # so a failed start does not leak the descriptor.
            if fault_file is not None:
                try:
                    fault_file.close()
                except OSError:
                    pass
            _FAULT_FILE = None
    sys.excepthook = _unhandled_exception
    threading.excepthook = _unhandled_thread


def shutdown_exception_logging() -> None:
    global _FAULT_FILE
    try:
        faulthandler.disable()
    except Exception:
        pass
    if _FAULT_FILE is not None:
        try:
            _FAULT_FILE.close()
        except OSError:
            pass
        _FAULT_FILE = None
    sys.excepthook = sys.__excepthook__
    threading.excepthook = threading.__excepthook__


def _acquire_mutex(name: str):
    """Создать named mutex. None, если приложение уже запущено."""
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.CreateMutexW.argtypes = (ctypes.c_void_p, ctypes.c_bool, ctypes.c_wchar_p)
    kernel32.CreateMutexW.restype = ctypes.c_void_p
    kernel32.CloseHandle.argtypes = (ctypes.c_void_p,)
    handle = kernel32.CreateMutexW(None, False, name)
    if not handle:
        return None
    if ctypes.get_last_error() == 183:  # ERROR_ALREADY_EXISTS
        kernel32.CloseHandle(handle)
        return None
    return handle


def _close_mutex(handle) -> None:
    if not handle:
        return
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.CloseHandle.argtypes = (ctypes.c_void_p,)
    kernel32.CloseHandle.restype = ctypes.c_bool
    kernel32.CloseHandle(handle)


def _stop_process_tree(process, timeout: float = 10.0) -> bool:
    """Stop the updated process together with its PyInstaller child.

    In onefile mode ``Popen`` returns the bootloader, which runs the real
    application as a child process. ``terminate()`` would kill only the
    bootloader, leaving the child alive and holding the tray mutex, so the
    whole tree is terminated with ``taskkill /T``.
    """
    if process.poll() is not None:
        return True

    try:
        subprocess.run(
            ["taskkill", "/PID", str(process.pid), "/T", "/F"],
            check=False,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            creationflags=CREATE_NO_WINDOW,
            timeout=timeout,
        )
    except (OSError, subprocess.SubprocessError):
        log.debug("taskkill failed for pid %s", process.pid, exc_info=True)

    try:
        process.wait(timeout=5.0)
        return True
    except subprocess.TimeoutExpired:
        pass

    # Last resort: kill the bootloader. The caller then checks the tray
    # mutex, which is the only reliable sign that the child is gone too.
    try:
        process.kill()
        process.wait(timeout=5.0)
        return True
    except (OSError, subprocess.TimeoutExpired):
        log.error("could not stop the updated instance (pid %s)", process.pid)
        return False


@lru_cache(maxsize=64)
def _font(size: int, condensed: bool = False) -> ImageFont.ImageFont:
    """Bold-шрифт для цифр. Для трёхзначных процентов — узкий Bahnschrift."""
    windir = Path(os.environ.get("WINDIR", r"C:\Windows"))
    fonts = windir / "Fonts"

    if condensed:
        path = fonts / "bahnschrift.ttf"
        if path.exists():
            try:
                font = ImageFont.truetype(str(path), size)
                font.set_variation_by_name("Bold Condensed")
                return font
            except (OSError, ValueError):
                pass

    for name in ("segoeuib.ttf", "arialbd.ttf"):
        path = fonts / name
        if path.exists():
            try:
                return ImageFont.truetype(str(path), size)
            except OSError:
                pass
    return ImageFont.load_default()


def _fit_icon_font(
    draw: ImageDraw.ImageDraw,
    text: str,
    icon_size: int,
    condensed: bool,
    fill_ratio: float,
) -> tuple[ImageFont.ImageFont, tuple[int, int, int, int]]:
    """Подобрать максимальный размер шрифта, вписанный в иконку."""
    max_side = icon_size * fill_ratio
    font_size = int(icon_size * 1.3)
    while font_size > 6:
        font = _font(font_size, condensed)
        bbox = draw.textbbox((0, 0), text, font=font)
        if bbox[2] - bbox[0] <= max_side and bbox[3] - bbox[1] <= max_side:
            return font, bbox
        font_size -= 1

    font = _font(6, condensed)
    return font, draw.textbbox((0, 0), text, font=font)


_RENDER_LOCK = threading.Lock()


def _use_condensed(text: str) -> bool:
    """Narrow font for multi-digit values so the digits stay large.

    A two-digit percentage in Segoe UI Bold is limited by the icon width and
    fills only about 62% of its height; Bahnschrift Bold Condensed fits the
    same width at roughly 88% of the height.
    """
    return text.isdigit() and len(text) >= 2


def render_icon(text: str, color: tuple[int, int, int, int], size: int = 64) -> Image.Image:
    """Иконка трея: максимально крупные цифры без обводки.

    Проценты из двух и трёх цифр набираются узким Bahnschrift Bold
    Condensed, одиночная цифра — обычным Segoe UI Bold: так цифры занимают
    почти всю высоту иконки и остаются читаемыми в трее.
    """
    with _RENDER_LOCK:
        image = Image.new("RGBA", (size, size), (0, 0, 0, 0))
        draw = ImageDraw.Draw(image)
        digits = text.isdigit()
        condensed = _use_condensed(text)
        fill_ratio = 0.98 if digits else 0.82

        font, bbox = _fit_icon_font(draw, text, size, condensed, fill_ratio)
        width = bbox[2] - bbox[0]
        height = bbox[3] - bbox[1]
        x = (size - width) / 2 - bbox[0]
        y = (size - height) / 2 - bbox[1]

        draw.text(
            (x, y),
            text,
            font=font,
            fill=color,
        )
        return image


def autostart_command() -> str:
    if getattr(sys, "frozen", False):
        return f'"{sys.executable}"'
    pythonw = Path(sys.executable).with_name("pythonw.exe")
    interpreter = pythonw if pythonw.exists() else Path(sys.executable)
    return f'"{interpreter}" "{Path(__file__).resolve()}"'


def _same_autostart_command(left: str, right: str) -> bool:
    """Compare autostart commands the way Windows resolves paths.

    A value written by an older build or with different capitalization still
    points at this executable, so the menu must not show it as disabled.
    """
    return os.path.normcase(left.strip()) == os.path.normcase(right.strip())


def autostart_enabled() -> bool:
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, RUN_KEY) as key:
            value, _ = winreg.QueryValueEx(key, RUN_VALUE)
        return _same_autostart_command(str(value), autostart_command())
    except OSError:
        return False


def set_autostart(enabled: bool) -> None:
    with winreg.CreateKey(winreg.HKEY_CURRENT_USER, RUN_KEY) as key:
        if enabled:
            winreg.SetValueEx(key, RUN_VALUE, 0, winreg.REG_SZ, autostart_command())
        else:
            try:
                winreg.DeleteValue(key, RUN_VALUE)
            except FileNotFoundError:
                pass


def _migrate_legacy_autostart() -> None:
    """Перенести автозапуск со старого имени LamzuThornBattery."""
    try:
        with winreg.OpenKey(
            winreg.HKEY_CURRENT_USER,
            RUN_KEY,
            0,
            winreg.KEY_READ | winreg.KEY_SET_VALUE,
        ) as key:
            try:
                winreg.QueryValueEx(key, LEGACY_RUN_VALUE)
            except FileNotFoundError:
                return
            try:
                winreg.QueryValueEx(key, RUN_VALUE)
            except FileNotFoundError:
                winreg.SetValueEx(
                    key, RUN_VALUE, 0, winreg.REG_SZ, autostart_command()
                )
            winreg.DeleteValue(key, LEGACY_RUN_VALUE)
            log.info("migrated autostart from %s", LEGACY_RUN_VALUE)
    except OSError:
        log.debug("could not migrate the legacy autostart entry", exc_info=True)


# ---------------------------------------------------------------------------
# Трей
# ---------------------------------------------------------------------------

class BatteryTray:
    def __init__(self, mutex=None) -> None:
        self._last: Optional[BatteryReading] = None
        self._last_ok: float = 0.0
        self._online = False
        self._state_lock = threading.Lock()
        self._stop = threading.Event()
        self._poll_lock = threading.Lock()
        self._icon_lock = threading.Lock()
        self._status_lock = threading.Lock()
        self._update_lock = threading.Lock()
        self._healthy = False
        self._cleanup_started = False
        self._health_timer: Optional[threading.Timer] = None
        self._handed_off = False
        self._update_pending = False
        self._mutex = mutex
        self._low_notified = False
        self._full_notified = False
        self._last_offered_version: Optional[str] = None
        self._available_update: Optional[updater.UpdateInfo] = None
        self._last_update_error_at = 0.0
        self._last_update_check_at = 0.0
        self._last_update_error: Optional[str] = None
        self._progress_percent: Optional[int] = None
        self._menu_signature: Optional[tuple[str, str]] = None
        self._light_theme = taskbar_uses_light_theme()
        settings = _load_settings()
        self._auto_update = (
            _normalize_update_mode(settings.get("update_mode")) == "auto"
        )
        self._icon = pystray.Icon(
            APP_ID,
            render_icon("?", palette(self._light_theme)["stale"]),
            APP_NAME,
            menu=self._build_menu(),
        )
        self._thread = threading.Thread(
            target=self._worker, name="battery-poller", daemon=True
        )
        self._watch_thread = threading.Thread(
            target=self._watch_devices, name="device-watcher", daemon=True
        )
        self._update_thread = threading.Thread(
            target=self._update_loop, name="update-checker", daemon=True
        )

    # --- меню -------------------------------------------------------------

    def _build_menu(self) -> pystray.Menu:
        return pystray.Menu(
            pystray.MenuItem(lambda item: self._display_name(), None, enabled=False),
            pystray.MenuItem(lambda item: self._menu_status(), None, enabled=False),
            pystray.Menu.SEPARATOR,
            pystray.MenuItem(
                lambda item: i18n.tr("menu.refresh"),
                self._on_refresh,
                default=True,
            ),
            pystray.Menu.SEPARATOR,
            pystray.MenuItem(
                lambda item: i18n.tr("menu.autostart"),
                self._on_toggle_autostart,
                checked=lambda item: autostart_enabled(),
            ),
            pystray.MenuItem(
                lambda item: i18n.tr("menu.logs"), self._on_open_logs
            ),
            pystray.MenuItem(
                lambda item: i18n.tr("menu.show_icon"),
                self._on_show_tray_icon,
            ),
            pystray.MenuItem(
                lambda item: i18n.tr("menu.release_page"),
                self._on_open_release_page,
            ),
            pystray.MenuItem(
                lambda item: i18n.tr("menu.auto_update"),
                self._on_toggle_auto_update,
                checked=lambda item: self._auto_update,
            ),
            pystray.MenuItem(
                lambda item: i18n.tr("menu.check_updates"),
                self._on_check_updates,
            ),
            pystray.MenuItem(
                lambda item: i18n.tr(
                    "menu.install_update",
                    version=(
                        self._available_update.version
                        if self._available_update
                        else ""
                    ),
                ),
                self._on_install_update,
                visible=lambda item: self._available_update is not None,
            ),
            pystray.MenuItem(
                lambda item: i18n.tr("menu.language"),
                self._language_menu(),
            ),
            pystray.Menu.SEPARATOR,
            pystray.MenuItem(lambda item: i18n.tr("menu.exit"), self._on_quit),
        )

    def _language_menu(self) -> pystray.Menu:
        """One radio item per supported language, built from ``i18n``."""

        def action(code):
            # pystray inspects the action's argument count, so the language
            # is bound in a closure instead of a default argument.
            def select(icon, item):
                self._on_set_language(icon, code)

            return select

        return pystray.Menu(
            *[
                pystray.MenuItem(
                    i18n.LANGUAGE_NAMES[code],
                    action(code),
                    checked=lambda item, code=code: (
                        i18n.get_language() == code
                    ),
                    radio=True,
                )
                for code in i18n.LANGUAGES
            ]
        )

    def _state_snapshot(
        self,
    ) -> tuple[Optional[BatteryReading], float, bool]:
        """Consistent (last reading, last success, online) for other threads."""
        with self._state_lock:
            return self._last, self._last_ok, self._online

    def _display_name(self) -> str:
        last, _, _ = self._state_snapshot()
        model = last.model if last else None
        return f"LAMZU {model}" if model else APP_NAME

    def _menu_status(self) -> str:
        last, last_ok, online = self._state_snapshot()
        if last is None:
            return i18n.tr("status.no_mouse")
        if online:
            state = i18n.state_text(last.state, last.percent)
            return f"{last.percent}% · {state}"
        age = i18n.human_age(time.time() - last_ok)
        return i18n.tr("status.offline", percent=last.percent, age=age)

    # --- опрос ------------------------------------------------------------

    def _worker(self) -> None:
        self._poll_safely()
        while not self._stop.wait(self._interval()):
            self._poll_safely()

    def _poll_safely(self) -> bool:
        """Keep the polling loop alive even if a poll raises unexpectedly."""
        try:
            return self._poll()
        except Exception:
            log.exception("battery poll failed")
            return False

    def _watch_devices(self) -> None:
        """Сразу реагировать на подключение кабеля/донгла, не опрашивая мышь."""
        signature = present_product_ids()
        while not self._stop.wait(5.0):
            current = present_product_ids()
            if current != signature:
                log.info(
                    "device change: %s -> %s",
                    sorted(signature),
                    sorted(current),
                )
                # Keep the old signature when the poll was skipped because
                # another one holds the lock: the change must not be lost.
                if self._poll_safely():
                    signature = current

    def _interval(self) -> float:
        last, _, online = self._state_snapshot()
        if last is None or not online:
            # Спящая/отключённая мышь или ещё нет первого ответа: пробуем
            # чаще, чтобы быстрее поймать пробуждение.
            return POLL_INTERVAL_OFFLINE
        if last.charging:
            return POLL_INTERVAL_CHARGING
        return POLL_INTERVAL

    def _poll(self) -> bool:
        """Read the battery once; False when another poll holds the lock."""
        if not self._poll_lock.acquire(timeout=5):
            return False
        try:
            reading = read_battery()
            if reading is not None:
                if reading != self._last:
                    name = f"LAMZU {reading.model}" if reading.model else APP_NAME
                    log.info(
                        "battery: %s%% (%s), %s, %s",
                        reading.percent,
                        i18n.state_text(reading.state, reading.percent),
                        name,
                        reading.protocol_text,
                    )
                with self._state_lock:
                    self._last = reading
                    self._last_ok = time.time()
                    self._online = True
                self._maybe_notify(reading)
            else:
                with self._state_lock:
                    was_online = self._online
                    self._online = False
                if was_online:
                    log.info("mouse did not respond (asleep or out of range)")
            self._apply()
            self._write_status()
            # Меню на Windows строится один раз, поэтому динамические
            # подписи (модель, процент) нужно обновлять вручную.
            self._sync_menu()
            return True
        finally:
            self._poll_lock.release()

    def _apply(self) -> None:
        # Re-read the theme on every poll: a switch between light and dark
        # mode is picked up within one interval without a WM_SETTINGCHANGE
        # hook, and the icon is rendered with the matching palette.
        self._light_theme = taskbar_uses_light_theme()
        colors = palette(self._light_theme)
        last, last_ok, online = self._state_snapshot()
        age = time.time() - last_ok if last else 0
        if last is None:
            text, color = "?", colors["stale"]
        elif online:
            text = str(last.percent)
            color = battery_color(
                last.percent,
                last.charging,
                last.full,
                self._light_theme,
            )
        else:
            # Показываем последнее значение приглушённым, пока оно не устарело.
            text = str(last.percent) if age < 900 else "?"
            color = colors["stale"]

        try:
            with self._icon_lock:
                self._icon.icon = render_icon(text, color)
                self._icon.title = self._tooltip()
        except Exception:
            log.exception("could not update tray icon")

    def _tooltip(self) -> str:
        last, last_ok, online = self._state_snapshot()
        name = self._display_name()
        if last is None:
            return i18n.tr("tooltip.no_connection", name=name)
        if online:
            state = i18n.state_text(last.state, last.percent)
            return f"{name}: {last.percent}% ({state})"
        age = i18n.human_age(time.time() - last_ok)
        return i18n.tr(
            "tooltip.offline",
            name=name,
            percent=last.percent,
            age=age,
        )

    def _maybe_notify(self, reading: BatteryReading) -> None:
        name = self._display_name()
        if reading.charging:
            # Пока идёт зарядка, низкий заряд не о чём предупреждать.
            self._low_notified = False
            self._full_notified = False
            return
        if reading.full:
            if not self._full_notified:
                delivered = self._notify(
                    i18n.tr(
                        "notify.full_body", name=name, percent=reading.percent
                    ),
                    i18n.tr("notify.full_title"),
                )
                # A failed notification is retried on the next poll.
                self._full_notified = delivered
            return

        # Hysteresis: replugging the cable at 99% must not repeat the
        # "charged" balloon, so the flag is cleared only when the charge
        # actually dropped.
        if reading.percent < 95:
            self._full_notified = False
        if reading.percent <= 15 and not self._low_notified:
            delivered = self._notify(
                i18n.tr(
                    "notify.low_body", name=name, percent=reading.percent
                ),
                i18n.tr("notify.low_title"),
            )
            self._low_notified = delivered
        elif reading.percent > 20:
            self._low_notified = False

    def _notify(self, message: str, title: str) -> bool:
        """Show a balloon; False when it could not be delivered."""
        if not getattr(self._icon, "visible", False):
            # The first poll runs before pystray finishes creating the icon.
            log.debug("notification skipped: the tray icon is not ready")
            return False
        try:
            self._icon.notify(message, title)
            return True
        except Exception:
            log.debug("notification failed", exc_info=True)
            return False

    def _menu_labels(self) -> tuple[str, str]:
        return (self._display_name(), self._menu_status())

    def _sync_menu(self) -> None:
        """Rebuild the tray menu only when its dynamic labels changed.

        pystray destroys and recreates the native menu on every update, and on
        Windows that can take the HMENU out from under an open popup. Polling
        runs every 20 seconds, so the menu is rebuilt only when the model or
        the status text really changed.
        """
        if self._menu_labels() != self._menu_signature:
            self._refresh_menu()

    def _refresh_menu(self) -> None:
        self._menu_signature = self._menu_labels()
        try:
            self._icon.update_menu()
        except Exception:
            log.debug("could not update tray menu", exc_info=True)

    # --- обновления -------------------------------------------------------

    def _on_check_updates(self, icon, item) -> None:
        threading.Thread(
            target=self._check_updates, args=(True,), daemon=True
        ).start()

    def _on_install_update(self, icon, item) -> None:
        if self._available_update is None:
            return
        threading.Thread(
            target=self._check_updates, args=(True,), daemon=True
        ).start()

    def _update_loop(self) -> None:
        # Дать трею появиться и не мешать первому опросу мыши.
        if self._stop.wait(8.0):
            return
        failures = 0
        while not self._stop.is_set():
            try:
                ok = self._check_updates(manual=False)
            except Exception:
                log.exception("update loop failed")
                ok = False
            failures, delay = _next_update_delay(ok, failures)
            if not self._stop.is_set():
                self._write_status()
            if self._stop.wait(delay):
                return

    def _on_toggle_auto_update(self, icon, item) -> None:
        self._auto_update = not self._auto_update
        _save_settings(
            update_mode="auto" if self._auto_update else "notify"
        )
        log.info("automatic updates = %s", self._auto_update)
        self._refresh_menu()

    def _on_open_release_page(self, icon, item) -> None:
        try:
            webbrowser.open(RELEASES_URL)
        except Exception:
            log.exception("could not open the release page")

    def _notify_update_error(
        self,
        manual: bool,
        exc: Optional[BaseException] = None,
        stage: str = "check",
    ) -> None:
        """Show a useful error without turning every offline start into spam."""
        detail = str(exc or "").strip()
        if len(detail) > 180:
            detail = detail[:177] + "..."
        now = time.time()
        if not manual:
            if now - self._last_update_error_at < 24 * 60 * 60:
                return
            self._last_update_error_at = now
        if manual or stage != "check":
            message = (
                i18n.tr("update.failed_reason", reason=detail)
                if detail
                else i18n.tr("update.failed")
            )
            self._notify(message, i18n.tr("update.title"))

    def _on_download_progress(self, done: int, total: int) -> None:
        percent = (
            max(0, min(100, int(done * 100 / total))) if total > 0 else None
        )
        if percent == self._progress_percent:
            return
        self._progress_percent = percent
        text = (
            i18n.tr("update.downloading", percent=percent)
            if percent is not None
            else i18n.tr("update.downloading_unknown")
        )
        try:
            with self._icon_lock:
                self._icon.title = f"{APP_NAME}: {text}"
        except Exception:
            log.debug("could not show update progress", exc_info=True)

    def _clear_progress(self) -> None:
        if self._progress_percent is None:
            return
        self._progress_percent = None
        self._apply()

    def _read_status(self) -> dict:
        try:
            data = json.loads(STATUS_FILE.read_text(encoding="utf-8-sig"))
        except (OSError, ValueError, TypeError):
            return {}
        return data if isinstance(data, dict) else {}

    def _release_instance_mutex(self) -> None:
        if self._mutex is None:
            return
        _close_mutex(self._mutex)
        self._mutex = None
        log.info("released the tray mutex for the next instance")

    def _reacquire_instance_mutex(self, timeout: float = 0.0) -> bool:
        """Take the tray mutex back after the new instance was stopped.

        ``timeout`` gives the killed child some time to release the mutex.
        Only once this returns True is it safe to touch the exe on disk.
        """
        if self._mutex is not None:
            return True
        deadline = time.monotonic() + max(0.0, timeout)
        while True:
            self._mutex = _acquire_mutex(MUTEX_TRAY)
            if self._mutex is not None:
                return True
            if time.monotonic() >= deadline:
                break
            time.sleep(0.2)
        log.error("could not reacquire the tray mutex after a failed update")
        return False

    def _wait_for_new_instance(
        self, process, version: str, started_at: float
    ) -> bool:
        """Wait until the updated process reports that its tray is healthy.

        The status file must belong to this hand-off: a leftover file from an
        earlier attempt that reported the same version and ``healthy: true``
        would otherwise be accepted as proof of a successful start.
        """
        deadline = time.monotonic() + UPDATE_READY_TIMEOUT
        not_before = int(started_at) - 5  # tolerate a small clock step
        while time.monotonic() < deadline:
            if self._stop.is_set():
                # The user asked to quit while the update was starting; let
                # the new instance take over instead of rolling it back.
                return True
            if process.poll() is not None:
                log.warning(
                    "updated instance exited early with code %s",
                    process.returncode,
                )
                return False
            status = self._read_status()
            if (
                updater.same_version(
                    str(status.get("version") or ""), version
                )
                and status.get("healthy") is True
                and _as_int(status.get("heartbeat_unix")) >= not_before
            ):
                return True
            time.sleep(0.4)
        log.warning("updated instance did not report readiness in time")
        return False

    def _recover_missing_exe(
        self,
        target: Path,
        backup_path: Path,
        new_path: Path,
        exc: BaseException,
    ) -> str:
        """Restore the exe after a failed replacement and return a reason.

        ``replace_executable`` normally restores the file itself, but when
        both replace operations fail the installation is left without
        ``LamzuBattery.exe`` and the backup is the only copy left.
        """
        if target.is_file():
            return str(exc)
        log.error("the executable is missing after the failed update")
        if updater.rollback_executable(target, backup_path, new_path):
            log.info("restored the previous version after a failed replacement")
            return (
                "the update could not be installed; "
                "the previous version was restored"
            )
        log.error(
            "the executable is missing and the backup could not be restored"
        )
        return (
            "the update failed and LamzuBattery.exe is missing; download "
            "the release from GitHub"
        )

    def _check_updates(self, manual: bool) -> bool:
        if not self._update_lock.acquire(blocking=False):
            if manual:
                self._notify(
                    i18n.tr("update.busy"), i18n.tr("update.title")
                )
            # Not a success: the loop must keep its retry counter, otherwise
            # a concurrent manual check would reset a pending backoff.
            return False
        try:
            # sys.executable указывает на реальный exe, даже если пользователь
            # перенёс программу в другую папку.
            target = updater.executable_path()
            if target is None:
                log.debug("update check skipped: running from source")
                if manual:
                    self._notify(
                        i18n.tr("update.source_mode"),
                        i18n.tr("update.title"),
                    )
                return True

            if manual:
                self._notify(
                    i18n.tr("update.checking"), i18n.tr("update.title")
                )
            self._last_update_check_at = time.time()
            try:
                info = updater.check_for_update(APP_VERSION)
            except Exception as exc:
                log.warning("update check failed: %s", exc)
                self._last_update_error = str(exc)
                self._notify_update_error(manual, exc, "check")
                return False

            self._last_update_error = None
            if info is None:
                self._available_update = None
                self._refresh_menu()
                log.debug("no update (current %s)", APP_VERSION)
                if manual:
                    self._notify(
                        i18n.tr("update.latest", version=APP_VERSION),
                        i18n.tr("update.title"),
                    )
                return True

            self._available_update = info
            self._refresh_menu()
            log.info(
                "update available: %s (current %s), executable %s",
                info.version,
                APP_VERSION,
                target,
            )
            if not manual and not self._auto_update:
                if self._last_offered_version != info.version:
                    self._last_offered_version = info.version
                    self._notify(
                        i18n.tr("update.available", version=info.version),
                        i18n.tr("update.title"),
                    )
                return True

            new_path, backup_path = updater.update_paths(target)
            if not updater.prepare_update_target(target, timeout=10.0):
                error = updater.UpdateError(
                    "the previous update backup is still being cleaned up"
                )
                log.warning("update delayed: %s", error)
                self._last_update_error = str(error)
                self._notify_update_error(manual, error, "preflight")
                return False

            try:
                updater.ensure_update_target(target, info.asset_size)
            except Exception as exc:
                log.warning("update preflight failed: %s", exc)
                self._last_update_error = str(exc)
                self._notify_update_error(manual, exc, "preflight")
                return False

            try:
                updater.download_update(
                    info,
                    new_path,
                    progress=self._on_download_progress,
                )
            except Exception as exc:
                log.warning("update download failed: %s", exc)
                self._last_update_error = str(exc)
                self._clear_progress()
                self._notify_update_error(manual, exc, "download")
                return False

            self._clear_progress()
            if self._stop.is_set():
                log.info("update cancelled: application is stopping")
                return True

            self._update_pending = True
            try:
                updater.replace_executable(target, new_path, backup_path)
            except Exception as exc:
                self._update_pending = False
                log.warning("update replacement failed: %s", exc)
                self._last_update_error = str(exc)
                self._notify_update_error(
                    manual,
                    self._recover_missing_exe(
                        target, backup_path, new_path, exc
                    ),
                    "install",
                )
                return False

            log.info("update installed, restarting %s", target)
            started_at = time.time()
            try:
                process = subprocess.Popen(
                    [str(target), "--wait-for-instance"],
                    cwd=str(target.parent),
                    creationflags=CREATE_NO_WINDOW,
                    # Без очистки _PYI_* новый exe унаследует временную
                    # папку старого bootloader и будет работать из неё.
                    env=updater.clean_environment(),
                )
            except Exception as exc:
                log.exception("could not start the updated application")
                restored = updater.rollback_executable(
                    target, backup_path, new_path
                )
                self._update_pending = False
                self._last_update_error = str(exc)
                reason = str(exc)
                if not restored:
                    reason = (
                        "the updated version could not be started and the "
                        "previous version could not be restored; download "
                        "the release from GitHub"
                    )
                self._notify_update_error(manual, reason, "install")
                return False

            self._notify(
                i18n.tr("update.installing", version=info.version),
                i18n.tr("update.title"),
            )
            # The old mutex must be released before the new process can start
            # its tray. The old process remains alive as a short watchdog.
            self._release_instance_mutex()
            if self._wait_for_new_instance(process, info.version, started_at):
                log.info("updated instance is healthy, stopping the old one")
                self._handed_off = True
                self._stop.set()
                self._icon.stop()
                return True

            log.error("updated instance failed the health check; rolling back")
            # The child of the bootloader is the real application: it has to
            # be gone before the files are replaced, otherwise it keeps
            # running from a renamed image and holds the tray mutex.
            stopped = _stop_process_tree(process)
            took_over = self._reacquire_instance_mutex(timeout=10.0)
            if not took_over:
                self._update_pending = False
                log.error(
                    "updated instance still owns the tray mutex; files left intact"
                )
                self._notify_update_error(
                    manual,
                    "the updated instance could not be stopped; "
                    "restart LAMZU Battery to finish the update",
                    "health",
                )
                return True

            restored = updater.rollback_executable(
                target, backup_path, new_path
            )
            self._update_pending = False
            if restored:
                log.info("restored the previous version at %s", target)
                # The restored instance needs the mutex; we keep running only
                # as a watchdog until it reports readiness.
                self._release_instance_mutex()
                try:
                    subprocess.Popen(
                        [str(target), "--wait-for-instance"],
                        cwd=str(target.parent),
                        creationflags=CREATE_NO_WINDOW,
                        env=updater.clean_environment(),
                    )
                    self._handed_off = True
                except Exception:
                    log.exception("could not restart the previous version")
                    self._reacquire_instance_mutex()
                reason = (
                    "the new version did not start correctly; "
                    "the previous version was restored"
                )
            else:
                log.error(
                    "the previous version could not be restored "
                    "(new instance stopped: %s)",
                    stopped,
                )
                reason = (
                    "the new version did not start correctly and the "
                    "previous version could not be restored; download "
                    "the release from GitHub"
                )
            self._notify_update_error(manual, reason, "health")
            if self._handed_off:
                time.sleep(0.5)
                self._stop.set()
                self._icon.stop()
            return True
        except Exception as exc:
            log.exception("update check crashed")
            self._last_update_error = str(exc)
            self._notify_update_error(manual, exc, "check")
            return False
        finally:
            self._update_lock.release()

    # --- файл состояния и health-check обновления ------------------------

    def _write_status(self, running: bool = True) -> None:
        if self._handed_off or self._update_pending:
            # During hand-off the new instance owns status.json; writing the
            # old version here would confuse its health check.
            return
        if running and self._stop.is_set():
            # A poll that finished during shutdown must not resurrect the
            # "running" marker after the final write.
            return
        last, last_ok, online = self._state_snapshot()
        payload = {
            "app": APP_NAME,
            "version": APP_VERSION,
            "pid": os.getpid(),
            "display_name": self._display_name(),
            "language": i18n.get_language(),
            "model": last.model if last else None,
            "protocol": last.protocol if last else None,
            "product": last.product if last else None,
            "running": running,
            "healthy": bool(self._healthy),
            "heartbeat_unix": int(time.time()),
            "online": online,
            "percent": last.percent if last else None,
            "state": last.state if last else None,
            "raw_percent": last.raw_percent if last else None,
            "voltage_mv": last.voltage_mv if last else None,
            "charging": bool(last and last.charging),
            "full": bool(last and last.full),
            "updated_unix": int(last_ok) if last else None,
            "polled_unix": int(time.time()),
            "update_last_check_unix": (
                int(self._last_update_check_at)
                if self._last_update_check_at
                else None
            ),
            "update_last_error": self._last_update_error,
        }
        tmp: Optional[Path] = None
        try:
            DATA_DIR.mkdir(parents=True, exist_ok=True)
            with self._status_lock:
                tmp = STATUS_FILE.with_name(
                    f"{STATUS_FILE.name}.{os.getpid()}.tmp"
                )
                tmp.write_text(
                    json.dumps(payload, ensure_ascii=False, indent=2),
                    encoding="utf-8",
                )
                os.replace(tmp, STATUS_FILE)
        except OSError:
            log.debug("could not write status file", exc_info=True)
            if tmp is not None:
                try:
                    tmp.unlink(missing_ok=True)
                except OSError:
                    pass

    def _mark_healthy(self) -> None:
        if self._stop.is_set():
            return
        self._healthy = True
        self._write_status()
        self._start_backup_cleanup()

    def _start_backup_cleanup(self) -> None:
        """Remove the rollback copy once the new version has proven itself.

        The copy is kept for ``BACKUP_GRACE_PERIOD`` on purpose: the previous
        version is the only way to recover from a defect that appears after
        the five second health check. If the application stops before the end
        of the grace period the copy stays on disk and is removed either by
        the next run or before the next update.
        """
        if self._cleanup_started:
            return
        self._cleanup_started = True
        executable = updater.executable_path()
        if executable is None:
            return
        threading.Thread(
            target=self._cleanup_backup_later,
            args=(executable,),
            name="update-cleanup",
            daemon=True,
        ).start()

    def _cleanup_backup_later(self, executable: Path) -> None:
        if self._stop.wait(BACKUP_GRACE_PERIOD):
            return
        updater.cleanup_backup(executable)
        log.info(
            "removed the rollback copy after %d minutes of uptime",
            BACKUP_GRACE_PERIOD // 60,
        )

    # --- действия меню ----------------------------------------------------

    def _on_refresh(self, icon, item) -> None:
        threading.Thread(target=self._poll, daemon=True).start()

    def _on_toggle_autostart(self, icon, item) -> None:
        try:
            set_autostart(not autostart_enabled())
            log.info("autostart = %s", autostart_enabled())
        except OSError:
            log.exception("could not change autostart")
        self._refresh_menu()

    def _on_open_logs(self, icon, item) -> None:
        try:
            os.startfile(DATA_DIR)  # noqa: S606
        except OSError:
            log.exception("could not open data folder")

    def _on_show_tray_icon(self, icon, item) -> None:
        threading.Thread(target=self._show_tray_icon, daemon=True).start()

    def _show_tray_icon(self) -> None:
        target = updater.executable_path()
        if target is None:
            self._notify(
                i18n.tr("tray.source_mode"),
                i18n.tr("tray.title"),
            )
            return
        result = tray_visibility.promote(target)
        log.info("tray promotion (manual): %s", result)
        if result in (tray_visibility.PROMOTED, tray_visibility.ALREADY):
            self._notify(
                i18n.tr("tray.pinned_body"),
                i18n.tr("tray.title"),
            )
            return
        self._notify(
            i18n.tr("tray.manual_body"),
            i18n.tr("tray.title"),
        )
        tray_visibility.open_taskbar_settings()

    def _on_set_language(self, icon, language: str) -> None:
        language = i18n.set_language(language)
        _save_language(language)
        log.info("language = %s", language)
        # Обновляем подсказку и все подписи меню.
        self._apply()
        self._write_status()
        self._refresh_menu()

    def _on_quit(self, icon, item) -> None:
        self._stop.set()
        icon.stop()

    # --- запуск -----------------------------------------------------------

    def _on_icon_ready(self, icon) -> None:
        icon.visible = True
        if getattr(sys, "frozen", False):
            threading.Thread(
                target=self._auto_promote_tray,
                name="tray-promoter",
                daemon=True,
            ).start()

    def _auto_promote_tray(self) -> None:
        target = updater.executable_path()
        if target is None or self._stop.is_set():
            return
        result = tray_visibility.promote_if_unconfigured(
            target, stop_event=self._stop
        )
        log.info("tray promotion (auto): %s", result)

    def run(self) -> None:
        log.info(
            "starting tray (%s %s) from %s",
            APP_NAME,
            APP_VERSION,
            updater.executable_path() or BASE_DIR,
        )
        self._write_status(running=True)
        self._health_timer = threading.Timer(HEALTH_DELAY, self._mark_healthy)
        self._health_timer.daemon = True
        self._health_timer.start()
        self._thread.start()
        self._watch_thread.start()
        self._update_thread.start()
        try:
            self._icon.run(setup=self._on_icon_ready)
        finally:
            self._stop.set()
            if self._health_timer is not None:
                self._health_timer.cancel()
            self._thread.join(timeout=2)
            self._watch_thread.join(timeout=1)
            self._update_thread.join(timeout=1)
            self._write_status(running=False)
            log.info("tray stopped")


# ---------------------------------------------------------------------------
# Точка входа
# ---------------------------------------------------------------------------

def main() -> int:
    i18n.set_language(_load_language())
    parser = argparse.ArgumentParser(description=APP_NAME)
    parser.add_argument(
        "--debug", action="store_true", help=i18n.tr("cli.debug_help")
    )
    parser.add_argument(
        "--version",
        action="version",
        version=APP_VERSION,
        help=i18n.tr("cli.version_help"),
    )
    parser.add_argument(
        "--wait-for-instance", action="store_true", help=argparse.SUPPRESS
    )
    args = parser.parse_args()

    # The crash hook goes first: a failure while setting up logging must be
    # reported instead of ending a windowed process silently.
    install_exception_logging()
    setup_logging(args.debug)
    log.info("data folder: %s", DATA_DIR)
    if _DATA_DIR_FALLBACK is not None:
        log.warning(
            "the folder next to the application is not writable, using %s",
            DATA_DIR,
        )
    executable = updater.executable_path()
    log.info(
        "LAMZU Battery %s, executable %s",
        APP_VERSION,
        executable or Path(__file__).resolve(),
    )
    _migrate_legacy_autostart()

    mutex = None
    if args.wait_for_instance:
        # Новый exe запускается до выхода старого процесса: ждём, пока
        # освободится mutex, вместо того чтобы сразу завершиться.
        deadline = time.monotonic() + 45.0
        while mutex is None and time.monotonic() < deadline:
            mutex = _acquire_mutex(MUTEX_TRAY)
            if mutex is None:
                time.sleep(0.4)
    else:
        mutex = _acquire_mutex(MUTEX_TRAY)
    if mutex is None:
        log.info("another instance is already running")
        shutdown_exception_logging()
        return 0

    if executable is not None:
        # A stale .new.exe is safe to remove before the tray starts, and a
        # rollback copy from an update more than a day old is not needed.
        # A fresh .old.exe is kept for BACKUP_GRACE_PERIOD instead.
        updater.cleanup_stale_backup(executable)
        updater.cleanup_download_files(executable, attempts=1)

    try:
        BatteryTray(mutex).run()
    except Exception:
        log.critical("fatal error", exc_info=True)
        _show_crash_message()
        return 1
    finally:
        shutdown_exception_logging()
    return 0


if __name__ == "__main__":
    sys.exit(main())
