import threading

import app
from lamzu import BatteryReading


def _tray():
    tray = object.__new__(app.BatteryTray)
    tray._last = None
    tray._last_ok = 0.0
    tray._online = False
    tray._state_lock = threading.Lock()
    return tray


def test_poll_intervals():
    tray = _tray()
    assert tray._interval() == app.POLL_INTERVAL_OFFLINE

    tray._last = BatteryReading(percent=50, state=0)
    tray._online = True
    assert app.POLL_INTERVAL == 5 * 60
    assert tray._interval() == 5 * 60

    tray._last = BatteryReading(percent=50, state=1)
    assert tray._interval() == app.POLL_INTERVAL_CHARGING


class _CountingIcon:
    def __init__(self):
        self.updates = 0

    def update_menu(self):
        self.updates += 1


def _tray_for_menu():
    tray = object.__new__(app.BatteryTray)
    tray._menu_signature = None
    tray._icon = _CountingIcon()
    tray._display_name = lambda: "LAMZU"
    tray._menu_status = lambda: "42%"
    return tray


def test_menu_is_rebuilt_only_when_labels_change():
    tray = _tray_for_menu()

    tray._sync_menu()
    tray._sync_menu()

    assert tray._icon.updates == 1

    tray._menu_status = lambda: "41%"
    tray._sync_menu()

    assert tray._icon.updates == 2


def test_refresh_menu_updates_the_signature():
    tray = _tray_for_menu()

    tray._refresh_menu()
    tray._sync_menu()

    assert tray._icon.updates == 1


class _BusyLock:
    def acquire(self, timeout=None):
        return False


def test_poll_reports_a_busy_lock():
    tray = _tray()
    tray._poll_lock = _BusyLock()

    assert tray._poll() is False


def test_poll_returns_true_after_a_reading(monkeypatch):
    tray = object.__new__(app.BatteryTray)
    tray._poll_lock = threading.Lock()
    tray._state_lock = threading.Lock()
    tray._last = None
    tray._last_ok = 0.0
    tray._online = False
    tray._apply = lambda: None
    tray._write_status = lambda running=True: None
    tray._sync_menu = lambda: None
    tray._maybe_notify = lambda reading: None
    monkeypatch.setattr(
        app, "read_battery", lambda: BatteryReading(percent=50, state=0)
    )

    assert tray._poll() is True
    assert tray._last.percent == 50
    assert tray._online is True


def test_poll_safely_keeps_the_loop_alive():
    tray = object.__new__(app.BatteryTray)

    def boom():
        raise RuntimeError("boom")

    tray._poll = boom

    assert tray._poll_safely() is False
