import json
import threading

import app


class FakeProcess:
    def __init__(self, returncode=None):
        self.returncode = returncode

    def poll(self):
        return self.returncode


def _tray_for_wait():
    tray = object.__new__(app.BatteryTray)
    tray._stop = threading.Event()
    return tray


def test_wait_for_new_instance_accepts_healthy_status(tmp_path, monkeypatch):
    monkeypatch.setattr(app, "STATUS_FILE", tmp_path / "status.json")
    monkeypatch.setattr(app, "UPDATE_READY_TIMEOUT", 1.0)
    (tmp_path / "status.json").write_text(
        json.dumps({"version": app.APP_VERSION, "healthy": True}),
        encoding="utf-8",
    )

    assert _tray_for_wait()._wait_for_new_instance(
        FakeProcess(), app.APP_VERSION
    )


def test_wait_for_new_instance_rejects_early_exit(tmp_path, monkeypatch):
    monkeypatch.setattr(app, "STATUS_FILE", tmp_path / "status.json")

    assert not _tray_for_wait()._wait_for_new_instance(
        FakeProcess(returncode=1), app.APP_VERSION
    )


def test_wait_for_new_instance_times_out(tmp_path, monkeypatch):
    monkeypatch.setattr(app, "STATUS_FILE", tmp_path / "status.json")
    monkeypatch.setattr(app, "UPDATE_READY_TIMEOUT", 0.15)

    assert not _tray_for_wait()._wait_for_new_instance(
        FakeProcess(), app.APP_VERSION
    )
