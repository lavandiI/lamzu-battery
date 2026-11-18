import json
import threading
import time

import app
from lamzu import BatteryReading


def _tray(tmp_path, monkeypatch, *, last=None, online=True, stopped=False):
    monkeypatch.setattr(app, "DATA_DIR", tmp_path)
    monkeypatch.setattr(app, "STATUS_FILE", tmp_path / "status.json")
    tray = object.__new__(app.BatteryTray)
    tray._state_lock = threading.Lock()
    tray._status_lock = threading.Lock()
    tray._stop = threading.Event()
    tray._handed_off = False
    tray._update_pending = False
    tray._healthy = True
    tray._last = last
    tray._last_ok = time.time()
    tray._online = online
    tray._last_update_check_at = 0.0
    tray._last_update_error = None
    if stopped:
        tray._stop.set()
    return tray


def test_state_snapshot_returns_the_same_reading():
    tray = object.__new__(app.BatteryTray)
    tray._state_lock = threading.Lock()
    reading = BatteryReading(percent=42, state=0)
    tray._last = reading
    tray._last_ok = 123.0
    tray._online = True

    assert tray._state_snapshot() == (reading, 123.0, True)


def test_status_is_not_rewritten_after_stop(tmp_path, monkeypatch):
    tray = _tray(tmp_path, monkeypatch, stopped=True)

    tray._write_status()
    assert not (tmp_path / "status.json").exists()

    tray._write_status(running=False)
    data = json.loads((tmp_path / "status.json").read_text(encoding="utf-8"))
    assert data["running"] is False


def test_status_payload_matches_one_reading(tmp_path, monkeypatch):
    reading = BatteryReading(
        percent=42,
        state=1,
        model="Thorn",
        protocol="compx",
        product="LAMZU THORN",
        voltage_mv=4000,
        raw_percent=40,
    )
    tray = _tray(tmp_path, monkeypatch, last=reading)

    tray._write_status()

    data = json.loads((tmp_path / "status.json").read_text(encoding="utf-8"))
    assert data["percent"] == 42
    assert data["model"] == "Thorn"
    assert data["protocol"] == "compx"
    assert data["charging"] is True
    assert data["online"] is True
    assert data["updated_unix"] == int(tray._last_ok)


def test_status_without_a_reading(tmp_path, monkeypatch):
    tray = _tray(tmp_path, monkeypatch, last=None, online=False)

    tray._write_status()

    data = json.loads((tmp_path / "status.json").read_text(encoding="utf-8"))
    assert data["percent"] is None
    assert data["online"] is False
    assert data["updated_unix"] is None


def test_status_leaves_no_temp_file(tmp_path, monkeypatch):
    tray = _tray(tmp_path, monkeypatch)

    tray._write_status()

    leftovers = [path.name for path in tmp_path.iterdir() if path.suffix == ".tmp"]
    assert leftovers == []


class _Icon:
    def __init__(self):
        self.icon = None
        self.title = None


def test_apply_renders_the_current_reading(tmp_path, monkeypatch):
    reading = BatteryReading(percent=99, state=0, model="Thorn")
    tray = _tray(tmp_path, monkeypatch, last=reading)
    tray._icon = _Icon()
    tray._icon_lock = threading.Lock()

    tray._apply()

    assert tray._icon.title.startswith("LAMZU Thorn: 99%")
    assert tray._icon.icon is not None

