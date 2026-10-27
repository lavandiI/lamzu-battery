"""LAMZU Battery — индикатор заряда мышей LAMZU в системном трее Windows.

Поддерживаются модели Lamzu с протоколами Compx и Aurora (см. lamzu.py).
Запуск:  pythonw app.py
Оверлей: pythonw app.py --overlay   (запускается из меню трея)
"""

from __future__ import annotations

import argparse
import ctypes
import json
import logging
import math
import os
import subprocess
import sys
import threading
import time
import winreg
from functools import lru_cache
from logging.handlers import RotatingFileHandler
from pathlib import Path
from typing import Optional

import i18n
import pystray
import updater
from PIL import Image, ImageDraw, ImageFont
from version import APP_VERSION

from lamzu import BatteryReading, present_product_ids, read_battery

APP_NAME = "LAMZU Battery"
APP_ID = "LamzuBattery"
MUTEX_TRAY = "Local\\LamzuBatteryTray"
MUTEX_OVERLAY = "Local\\LamzuBatteryOverlay"
RUN_KEY = r"Software\Microsoft\Windows\CurrentVersion\Run"
RUN_VALUE = "LamzuBattery"
LEGACY_RUN_VALUE = "LamzuThornBattery"
CREATE_NO_WINDOW = 0x08000000

if getattr(sys, "frozen", False):
    # PyInstaller: файлы рядом с exe, а не во временной папке _MEIPASS.
    BASE_DIR = Path(sys.executable).resolve().parent
else:
    BASE_DIR = Path(__file__).resolve().parent


def _pick_data_dir() -> Path:
    """Портативная папка data рядом с приложением, иначе LocalAppData.

    Важно: Python из Microsoft Store виртуализирует запись в AppData,
    поэтому основной вариант — обычная папка рядом с программой.
    """
    preferred = BASE_DIR / "data"
    try:
        preferred.mkdir(parents=True, exist_ok=True)
        probe = preferred / ".write_test"
        probe.write_text("ok", encoding="ascii")
        probe.unlink()
        return preferred
    except OSError:
        fallback = Path(os.environ.get("LOCALAPPDATA", Path.home())) / APP_ID
        fallback.mkdir(parents=True, exist_ok=True)
        return fallback


DATA_DIR = _pick_data_dir()
STATUS_FILE = DATA_DIR / "status.json"
OVERLAY_POS_FILE = DATA_DIR / "overlay.json"
SETTINGS_FILE = DATA_DIR / "settings.json"
LOG_FILE = DATA_DIR / "app.log"

COLOR_NORMAL = (0, 122, 255, 255)       # синий
COLOR_WARN = (255, 214, 10, 255)        # яркий жёлтый
COLOR_LOW = (255, 59, 48, 255)          # яркий красный
COLOR_CHARGE = (0, 140, 70, 255)        # тёмно-зелёный
COLOR_STALE = (184, 212, 255, 255)      # светло-голубой вместо тусклого серого

# Обычное обновление — раз в 30 минут. Во время зарядки чаще, чтобы
# не пропустить 100%; после неудачного опроса — каждые 20 секунд, чтобы
# быстрее поймать проснувшуюся мышь.
POLL_INTERVAL = 30 * 60
POLL_INTERVAL_CHARGING = 60
POLL_INTERVAL_OFFLINE = 20

log = logging.getLogger("lamzu_tray")


def _load_language() -> str:
    """Прочитать выбранный язык; при первом запуске — язык системы."""
    try:
        data = json.loads(SETTINGS_FILE.read_text(encoding="utf-8"))
        language = data.get("language")
    except (OSError, ValueError, TypeError, AttributeError):
        language = None
    return i18n.normalize_language(language or i18n.detect_system_language())


def _save_language(language: str) -> None:
    try:
        DATA_DIR.mkdir(parents=True, exist_ok=True)
        tmp = SETTINGS_FILE.with_suffix(".tmp")
        tmp.write_text(
            json.dumps({"language": language}, ensure_ascii=False, indent=2),
            encoding="utf-8",
        )
        os.replace(tmp, SETTINGS_FILE)
    except OSError:
        log.debug("could not save settings", exc_info=True)


# ---------------------------------------------------------------------------
# Общие помощники
# ---------------------------------------------------------------------------

def setup_logging(debug: bool = False) -> None:
    DATA_DIR.mkdir(parents=True, exist_ok=True)
    handler = RotatingFileHandler(
        LOG_FILE, maxBytes=256 * 1024, backupCount=1, encoding="utf-8"
    )
    handler.setFormatter(
        logging.Formatter("%(asctime)s %(levelname)s %(message)s", "%Y-%m-%d %H:%M:%S")
    )
    root = logging.getLogger()
    root.setLevel(logging.DEBUG if debug else logging.INFO)
    root.addHandler(handler)
    # Не засорять журнал внутренними сообщениями библиотек.
    for noisy in ("PIL", "pystray", "hid"):
        logging.getLogger(noisy).setLevel(logging.INFO)
    if debug and sys.stderr is not None:
        console = logging.StreamHandler()
        console.setFormatter(handler.formatter)
        root.addHandler(console)


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


def render_icon(text: str, color: tuple[int, int, int, int], size: int = 64) -> Image.Image:
    """Иконка трея: максимально крупные цифры без обводки.

    Двузначные проценты набираются обычным Segoe UI Bold, трёхзначные —
    узким Bahnschrift Bold Condensed: так «100» тоже занимает почти всю
    высоту иконки, не превращаясь в нечитаемую мелочь.
    """
    with _RENDER_LOCK:
        image = Image.new("RGBA", (size, size), (0, 0, 0, 0))
        draw = ImageDraw.Draw(image)
        digits = text.isdigit()
        condensed = len(text) >= 3 and digits
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


def _rgba_hex(color: tuple[int, int, int, int]) -> str:
    """Цвет PIL (RGBA) -> строка #RRGGBB для tkinter."""
    return f"#{color[0]:02X}{color[1]:02X}{color[2]:02X}"


def _status_color(percent: Optional[int], state: Optional[int], online: bool) -> str:
    if percent is None or not online:
        return _rgba_hex(COLOR_STALE)
    if state in (1, 2):
        return _rgba_hex(COLOR_CHARGE)
    if percent <= 15:
        return _rgba_hex(COLOR_LOW)
    if percent <= 30:
        return _rgba_hex(COLOR_WARN)
    return _rgba_hex(COLOR_NORMAL)


def autostart_command() -> str:
    if getattr(sys, "frozen", False):
        return f'"{sys.executable}"'
    pythonw = Path(sys.executable).with_name("pythonw.exe")
    interpreter = pythonw if pythonw.exists() else Path(sys.executable)
    return f'"{interpreter}" "{Path(__file__).resolve()}"'


def autostart_enabled() -> bool:
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, RUN_KEY) as key:
            value, _ = winreg.QueryValueEx(key, RUN_VALUE)
        return str(value) == autostart_command()
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
        pass


def overlay_command() -> list[str]:
    if getattr(sys, "frozen", False):
        return [sys.executable, "--overlay"]
    pythonw = Path(sys.executable).with_name("pythonw.exe")
    interpreter = pythonw if pythonw.exists() else Path(sys.executable)
    return [str(interpreter), str(Path(__file__).resolve()), "--overlay"]


# ---------------------------------------------------------------------------
# Трей
# ---------------------------------------------------------------------------

class BatteryTray:
    def __init__(self) -> None:
        self._last: Optional[BatteryReading] = None
        self._last_ok: float = 0.0
        self._online = False
        self._stop = threading.Event()
        self._poll_lock = threading.Lock()
        self._icon_lock = threading.Lock()
        self._update_lock = threading.Lock()
        self._overlay: Optional[subprocess.Popen] = None
        self._low_notified = False
        self._full_notified = False
        self._icon = pystray.Icon(
            APP_ID,
            render_icon("?", COLOR_STALE),
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
            target=self._startup_update_check, name="update-checker", daemon=True
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
            pystray.MenuItem(
                lambda item: i18n.tr("menu.desktop"),
                self._on_toggle_overlay,
                checked=lambda item: self._overlay_running(),
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
                lambda item: i18n.tr("menu.check_updates"),
                self._on_check_updates,
            ),
            pystray.MenuItem(
                lambda item: i18n.tr("menu.language"),
                pystray.Menu(
                    pystray.MenuItem(
                        i18n.LANGUAGE_NAMES["ru"],
                        lambda icon, item: self._on_set_language(icon, "ru"),
                        checked=lambda item: i18n.get_language() == "ru",
                        radio=True,
                    ),
                    pystray.MenuItem(
                        i18n.LANGUAGE_NAMES["en"],
                        lambda icon, item: self._on_set_language(icon, "en"),
                        checked=lambda item: i18n.get_language() == "en",
                        radio=True,
                    ),
                ),
            ),
            pystray.Menu.SEPARATOR,
            pystray.MenuItem(lambda item: i18n.tr("menu.exit"), self._on_quit),
        )

    def _display_name(self) -> str:
        model = self._last.model if self._last else None
        return f"LAMZU {model}" if model else APP_NAME

    def _menu_status(self) -> str:
        if self._last is None:
            return i18n.tr("status.no_mouse")
        if self._online:
            state = i18n.state_text(self._last.state, self._last.percent)
            return f"{self._last.percent}% · {state}"
        age = i18n.human_age(time.time() - self._last_ok)
        return i18n.tr(
            "status.offline", percent=self._last.percent, age=age
        )

    # --- опрос ------------------------------------------------------------

    def _worker(self) -> None:
        self._poll()
        while not self._stop.wait(self._interval()):
            self._poll()

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
                signature = current
                self._poll()

    def _interval(self) -> float:
        if self._last is None or not self._online:
            # Спящая/отключённая мышь или ещё нет первого ответа: пробуем
            # чаще, чтобы быстрее поймать пробуждение.
            return POLL_INTERVAL_OFFLINE
        if self._last.charging:
            return POLL_INTERVAL_CHARGING
        return POLL_INTERVAL

    def _poll(self) -> None:
        if not self._poll_lock.acquire(timeout=5):
            return
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
                self._last = reading
                self._last_ok = time.time()
                self._online = True
                self._maybe_notify(reading)
            else:
                if self._online:
                    log.info("mouse did not respond (asleep or out of range)")
                self._online = False
            self._apply()
            self._write_status()
            # Меню на Windows строится один раз, поэтому динамические
            # подписи (модель, процент) нужно обновлять вручную.
            try:
                self._icon.update_menu()
            except Exception:
                log.debug("could not update tray menu", exc_info=True)
        finally:
            self._poll_lock.release()

    def _apply(self) -> None:
        age = time.time() - self._last_ok if self._last else 0
        if self._last is None:
            text, color = "?", COLOR_STALE
        elif self._online:
            text = str(self._last.percent)
            if self._last.charging or self._last.full:
                color = COLOR_CHARGE
            elif self._last.percent <= 15:
                color = COLOR_LOW
            elif self._last.percent <= 30:
                color = COLOR_WARN
            else:
                color = COLOR_NORMAL
        else:
            # Показываем последнее значение светло-голубым, пока оно не устарело.
            text = str(self._last.percent) if age < 900 else "?"
            color = COLOR_STALE

        try:
            with self._icon_lock:
                self._icon.icon = render_icon(text, color)
                self._icon.title = self._tooltip()
        except Exception:
            log.exception("could not update tray icon")

    def _tooltip(self) -> str:
        name = self._display_name()
        if self._last is None:
            return i18n.tr("tooltip.no_connection", name=name)
        if self._online:
            state = i18n.state_text(self._last.state, self._last.percent)
            return f"{name}: {self._last.percent}% ({state})"
        age = i18n.human_age(time.time() - self._last_ok)
        return i18n.tr(
            "tooltip.offline",
            name=name,
            percent=self._last.percent,
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
                self._notify(
                    i18n.tr(
                        "notify.full_body", name=name, percent=reading.percent
                    ),
                    i18n.tr("notify.full_title"),
                )
                self._full_notified = True
            return

        self._full_notified = False
        if reading.percent <= 15 and not self._low_notified:
            self._notify(
                i18n.tr(
                    "notify.low_body", name=name, percent=reading.percent
                ),
                i18n.tr("notify.low_title"),
            )
            self._low_notified = True
        elif reading.percent > 20:
            self._low_notified = False

    def _notify(self, message: str, title: str) -> None:
        try:
            self._icon.notify(message, title)
        except Exception:
            log.debug("notification failed", exc_info=True)

    # --- обновления -------------------------------------------------------

    def _on_check_updates(self, icon, item) -> None:
        threading.Thread(
            target=self._check_updates, args=(True,), daemon=True
        ).start()

    def _startup_update_check(self) -> None:
        # Дать трею появиться и не мешать первому опросу мыши.
        if self._stop.wait(8.0):
            return
        self._check_updates(manual=False)

    def _notify_update_error(self, manual: bool) -> None:
        if manual:
            self._notify(
                i18n.tr("update.failed"), i18n.tr("update.title")
            )

    def _check_updates(self, manual: bool) -> None:
        if not self._update_lock.acquire(blocking=False):
            if manual:
                self._notify(
                    i18n.tr("update.busy"), i18n.tr("update.title")
                )
            return
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
                return

            if manual:
                self._notify(
                    i18n.tr("update.checking"), i18n.tr("update.title")
                )
            try:
                info = updater.check_for_update(APP_VERSION)
            except Exception as exc:
                log.warning("update check failed: %s", exc)
                self._notify_update_error(manual)
                return

            if info is None:
                log.debug("no update (current %s)", APP_VERSION)
                if manual:
                    self._notify(
                        i18n.tr("update.latest", version=APP_VERSION),
                        i18n.tr("update.title"),
                    )
                return

            log.info(
                "update available: %s (current %s), executable %s",
                info.version,
                APP_VERSION,
                target,
            )
            new_path, backup_path = updater.update_paths(target)
            try:
                updater.download_update(info, new_path)
            except Exception as exc:
                log.warning("update download failed: %s", exc)
                self._notify_update_error(manual)
                return

            if self._stop.is_set():
                log.info("update cancelled: application is stopping")
                return

            try:
                updater.replace_executable(target, new_path, backup_path)
            except Exception as exc:
                log.warning("update replacement failed: %s", exc)
                self._notify_update_error(manual)
                return

            log.info("update installed, restarting %s", target)
            try:
                subprocess.Popen(
                    [str(target), "--wait-for-instance"],
                    cwd=str(target.parent),
                    creationflags=CREATE_NO_WINDOW,
                    # Без очистки _PYI_* новый exe унаследует временную
                    # папку старого bootloader и будет работать из неё.
                    env=updater.clean_environment(),
                )
            except Exception:
                log.exception("could not start the updated application")
                updater.rollback_executable(target, backup_path, new_path)
                self._notify_update_error(manual)
                return

            self._notify(
                i18n.tr("update.installing", version=info.version),
                i18n.tr("update.title"),
            )
            time.sleep(1.5)
            self._stop.set()
            self._icon.stop()
        finally:
            self._update_lock.release()

    # --- файл состояния для оверлея --------------------------------------

    def _write_status(self, running: bool = True) -> None:
        payload = {
            "app": APP_NAME,
            "version": APP_VERSION,
            "display_name": self._display_name(),
            "language": i18n.get_language(),
            "model": self._last.model if self._last else None,
            "protocol": self._last.protocol if self._last else None,
            "product": self._last.product if self._last else None,
            "running": running,
            "online": self._online,
            "percent": self._last.percent if self._last else None,
            "state": self._last.state if self._last else None,
            "state_text": (
                i18n.state_text(self._last.state, self._last.percent)
                if self._last
                else None
            ),
            "charging": bool(self._last and self._last.charging),
            "full": bool(self._last and self._last.full),
            "updated_unix": int(self._last_ok) if self._last else None,
            "polled_unix": int(time.time()),
            # Пока приложение работает, показание остаётся свежим до
            # следующего опроса (30 мин в обычном режиме, 1 мин на зарядке).
            "stale_after": int(max(180, self._interval() * 1.5 + 60)),
        }
        try:
            DATA_DIR.mkdir(parents=True, exist_ok=True)
            tmp = STATUS_FILE.with_suffix(".tmp")
            tmp.write_text(
                json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8"
            )
            os.replace(tmp, STATUS_FILE)
        except OSError:
            log.debug("could not write status file", exc_info=True)

    # --- действия меню ----------------------------------------------------

    def _on_refresh(self, icon, item) -> None:
        threading.Thread(target=self._poll, daemon=True).start()

    def _overlay_running(self) -> bool:
        return self._overlay is not None and self._overlay.poll() is None

    def _on_toggle_overlay(self, icon, item) -> None:
        if self._overlay_running():
            self._overlay.terminate()
            try:
                self._overlay.wait(timeout=2)
            except Exception:
                pass
            self._overlay = None
            log.info("overlay stopped")
        else:
            try:
                self._overlay = subprocess.Popen(
                    overlay_command(),
                    cwd=str(BASE_DIR),
                    creationflags=CREATE_NO_WINDOW,
                )
                log.info("overlay started (pid %s)", self._overlay.pid)
            except Exception:
                log.exception("could not start overlay")
        try:
            icon.update_menu()
        except Exception:
            pass

    def _on_toggle_autostart(self, icon, item) -> None:
        try:
            set_autostart(not autostart_enabled())
            log.info("autostart = %s", autostart_enabled())
        except OSError:
            log.exception("could not change autostart")
        try:
            icon.update_menu()
        except Exception:
            pass

    def _on_open_logs(self, icon, item) -> None:
        try:
            os.startfile(DATA_DIR)  # noqa: S606
        except OSError:
            log.exception("could not open data folder")

    def _on_set_language(self, icon, language: str) -> None:
        language = i18n.set_language(language)
        _save_language(language)
        log.info("language = %s", language)
        # Обновляем подсказку, состояние для оверлея и все подписи меню.
        self._apply()
        self._write_status()
        try:
            icon.update_menu()
        except Exception:
            log.debug("could not update tray menu", exc_info=True)

    def _on_quit(self, icon, item) -> None:
        self._stop.set()
        icon.stop()

    def _kill_overlay(self) -> None:
        if self._overlay_running():
            try:
                self._overlay.terminate()
                self._overlay.wait(timeout=2)
            except Exception:
                pass
        self._overlay = None

    # --- запуск -----------------------------------------------------------

    def run(self) -> None:
        log.info(
            "starting tray (%s %s) from %s",
            APP_NAME,
            APP_VERSION,
            updater.executable_path() or BASE_DIR,
        )
        self._write_status(running=True)
        self._thread.start()
        self._watch_thread.start()
        self._update_thread.start()
        try:
            self._icon.run()
        finally:
            self._stop.set()
            self._thread.join(timeout=2)
            self._watch_thread.join(timeout=1)
            self._update_thread.join(timeout=1)
            self._kill_overlay()
            self._write_status(running=False)
            log.info("tray stopped")


# ---------------------------------------------------------------------------
# Оверлей на рабочем столе
# ---------------------------------------------------------------------------

def run_overlay() -> int:
    import tkinter as tk

    i18n.set_language(_load_language())
    mutex = _acquire_mutex(MUTEX_OVERLAY)
    if mutex is None:
        return 0

    width, height = 190, 60
    chroma = "#010203"
    bg = "#141418"
    border = "#2C2C31"

    root = tk.Tk()
    root.title(APP_NAME)
    root.overrideredirect(True)
    root.attributes("-topmost", True)
    root.attributes("-alpha", 0.94)
    try:
        root.attributes("-transparentcolor", chroma)
    except tk.TclError:
        pass
    root.configure(bg=chroma)

    saved = _load_overlay_pos()
    if saved:
        x, y = saved
    else:
        x = root.winfo_screenwidth() - width - 24
        y = 24
    x = max(0, min(x, root.winfo_screenwidth() - width))
    y = max(0, min(y, root.winfo_screenheight() - height))
    root.geometry(f"{width}x{height}+{x}+{y}")

    canvas = tk.Canvas(
        root,
        width=width,
        height=height,
        bg=chroma,
        highlightthickness=0,
        bd=0,
    )
    canvas.pack(fill="both", expand=True)

    # Скруглённый фон
    r = 12
    points = [
        r, 0, width - r, 0, width, 0, width, r,
        width, height - r, width, height, width - r, height,
        r, height, 0, height, 0, height - r, 0, r, 0, 0,
    ]
    canvas.create_polygon(points, smooth=True, fill=bg, outline=border, width=1)

    header_item = canvas.create_text(
        (14, 7), anchor="nw", text=APP_NAME.upper(), fill="#8E8E93",
        font=("Segoe UI Semibold", 7),
    )
    percent_item = canvas.create_text(
        (12, 14), anchor="nw", text="—", fill="#B8D4FF",
        font=("Segoe UI Bold", 26),
    )
    state_item = canvas.create_text(
        (105, 19), anchor="nw", text=i18n.tr("overlay.no_connection"),
        fill="#B8D4FF", font=("Segoe UI", 8),
    )

    # Батарейка
    canvas.create_rectangle(105, 38, 166, 48, outline="#48484A", width=1)
    canvas.create_rectangle(166, 41, 169, 45, outline="#48484A", fill="#48484A")
    bar_item = canvas.create_rectangle(107, 40, 107, 46, outline="", fill="#B8D4FF")
    # Бегущий блик во время зарядки
    dot_item = canvas.create_oval(
        0, 0, 0, 0, outline="", fill="#F2FFF7", state="hidden"
    )

    # Перетаскивание
    drag = {"x": 0, "y": 0}

    def start_drag(event):
        drag["x"] = event.x_root - root.winfo_x()
        drag["y"] = event.y_root - root.winfo_y()

    def move_drag(event):
        nx = event.x_root - drag["x"]
        ny = event.y_root - drag["y"]
        root.geometry(f"+{nx}+{ny}")

    for widget in (root, canvas):
        widget.bind("<Button-1>", start_drag)
        widget.bind("<B1-Motion>", move_drag)

    def close_overlay():
        _save_overlay_pos(root.winfo_x(), root.winfo_y())
        root.destroy()

    menu = tk.Menu(root, tearoff=0)
    menu.add_command(label=i18n.tr("overlay.close"), command=close_overlay)

    def popup(event):
        try:
            menu.tk_popup(event.x_root, event.y_root)
        finally:
            menu.grab_release()

    root.bind("<Button-3>", popup)
    canvas.bind("<Button-3>", popup)

    started = time.time()
    missing_since: Optional[float] = None
    stopped_since: Optional[float] = None
    cache = {"data": None, "read_at": 0.0}

    def tick():
        nonlocal missing_since, stopped_since
        now = time.time()

        if now - cache["read_at"] >= 1.0:
            cache["read_at"] = now
            try:
                data = json.loads(STATUS_FILE.read_text(encoding="utf-8"))
                missing_since = None
            except Exception:
                data = None
                if missing_since is None:
                    missing_since = now

            if data is None:
                cache["data"] = None
                if now - started > 10:
                    close_overlay()
                    return
            else:
                if not data.get("running", True):
                    if stopped_since is None:
                        stopped_since = now
                    elif now - stopped_since > 5:
                        close_overlay()
                        return
                else:
                    stopped_since = None
                cache["data"] = data

        data = cache["data"]
        if data is not None:
            language = data.get("language")
            if language in i18n.LANGUAGES and language != i18n.get_language():
                i18n.set_language(language)
                try:
                    menu.entryconfigure(0, label=i18n.tr("overlay.close"))
                except tk.TclError:
                    pass

        if data is None:
            percent, state, online = None, None, False
            charging = full = False
            state_text = i18n.tr("overlay.no_data")
            display_name = APP_NAME
        else:
            percent = data.get("percent")
            state = data.get("state")
            online = bool(data.get("online"))
            charging = bool(data.get("charging"))
            full = bool(data.get("full"))
            display_name = data.get("display_name") or APP_NAME
            age = now - (data.get("updated_unix") or 0)
            stale_after = data.get("stale_after") or 180
            if online and age > stale_after:
                online = False
                charging = False
                full = False
                state_text = i18n.tr("overlay.stale")
            elif not online:
                charging = False
                full = False
                state_text = i18n.tr("overlay.no_connection")
            elif state is not None:
                state_text = i18n.state_text(state, percent)
            else:
                state_text = i18n.tr("overlay.no_data")

        color = _status_color(percent, state, online)
        phase = 0.0
        if online and charging:
            phase = (math.sin(now * 2 * math.pi / 1.6) + 1) / 2

        canvas.itemconfigure(header_item, text=display_name.upper())
        canvas.itemconfigure(
            percent_item,
            text="—" if percent is None else f"{percent}%",
            fill=color,
        )
        canvas.itemconfigure(state_item, text=state_text, fill=color)

        if percent is None:
            bar_w = 0
        else:
            bar_w = int(57 * max(0, min(percent, 100)) / 100)
        canvas.coords(bar_item, 107, 40, 107 + bar_w, 46)
        canvas.itemconfigure(bar_item, fill=color)

        if online and charging:
            # Блик бежит по заполненной части батарейки.
            travel = max(0, bar_w - 5)
            dot_x = 107 + travel * phase
            canvas.coords(dot_item, dot_x, 40.5, dot_x + 5, 45.5)
            canvas.itemconfigure(dot_item, state="normal")
        else:
            canvas.itemconfigure(dot_item, state="hidden")

        root.after(80 if (online and charging) else 500, tick)

    tick()
    root.mainloop()
    return 0


def _load_overlay_pos() -> Optional[tuple[int, int]]:
    try:
        data = json.loads(OVERLAY_POS_FILE.read_text(encoding="utf-8"))
        return int(data["x"]), int(data["y"])
    except Exception:
        return None


def _save_overlay_pos(x: int, y: int) -> None:
    try:
        DATA_DIR.mkdir(parents=True, exist_ok=True)
        OVERLAY_POS_FILE.write_text(
            json.dumps({"x": int(x), "y": int(y)}), encoding="utf-8"
        )
    except OSError:
        pass


# ---------------------------------------------------------------------------
# Точка входа
# ---------------------------------------------------------------------------

def main() -> int:
    i18n.set_language(_load_language())
    parser = argparse.ArgumentParser(description=APP_NAME)
    parser.add_argument(
        "--overlay", action="store_true", help=i18n.tr("cli.overlay_help")
    )
    parser.add_argument(
        "--debug", action="store_true", help=i18n.tr("cli.debug_help")
    )
    parser.add_argument(
        "--wait-for-instance", action="store_true", help=argparse.SUPPRESS
    )
    args = parser.parse_args()

    if args.overlay:
        return run_overlay()

    setup_logging(args.debug)
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
        return 0

    if executable is not None:
        # Скачанный файл и .part можно убрать сразу, а старый exe может быть
        # ещё занят bootloader'ом предыдущей версии — его удаляем в фоне,
        # не задерживая появление трея.
        updater.cleanup_update_files(executable, attempts=1)
        threading.Thread(
            target=updater.cleanup_backup,
            args=(executable,),
            name="update-cleanup",
            daemon=True,
        ).start()

    BatteryTray().run()
    return 0


if __name__ == "__main__":
    sys.exit(main())
