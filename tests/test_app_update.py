import json
import re
import threading
import time

import app
import updater


class FakeProcess:
    def __init__(self, returncode=None):
        self.returncode = returncode

    def poll(self):
        return self.returncode


def _tray_for_wait():
    tray = object.__new__(app.BatteryTray)
    tray._stop = threading.Event()
    return tray


def _write_status(path, version, healthy=True, heartbeat=None):
    if heartbeat is None:
        heartbeat = int(time.time())
    path.write_text(
        json.dumps(
            {
                "version": version,
                "healthy": healthy,
                "heartbeat_unix": heartbeat,
            }
        ),
        encoding="utf-8",
    )


def test_wait_for_new_instance_accepts_healthy_status(tmp_path, monkeypatch):
    status_file = tmp_path / "status.json"
    monkeypatch.setattr(app, "STATUS_FILE", status_file)
    monkeypatch.setattr(app, "UPDATE_READY_TIMEOUT", 1.0)
    _write_status(status_file, app.APP_VERSION)

    assert _tray_for_wait()._wait_for_new_instance(
        FakeProcess(), app.APP_VERSION, time.time()
    )


def test_wait_for_new_instance_accepts_normalized_version(
    tmp_path, monkeypatch
):
    status_file = tmp_path / "status.json"
    monkeypatch.setattr(app, "STATUS_FILE", status_file)
    monkeypatch.setattr(app, "UPDATE_READY_TIMEOUT", 1.0)
    _write_status(status_file, "1.6.0")

    assert _tray_for_wait()._wait_for_new_instance(
        FakeProcess(), "1.6", time.time()
    )


def test_wait_for_new_instance_accepts_status_with_bom(tmp_path, monkeypatch):
    status_file = tmp_path / "status.json"
    monkeypatch.setattr(app, "STATUS_FILE", status_file)
    monkeypatch.setattr(app, "UPDATE_READY_TIMEOUT", 1.0)
    status_file.write_text(
        json.dumps(
            {
                "version": app.APP_VERSION,
                "healthy": True,
                "heartbeat_unix": int(time.time()),
            }
        ),
        encoding="utf-8-sig",
    )

    assert _tray_for_wait()._wait_for_new_instance(
        FakeProcess(), app.APP_VERSION, time.time()
    )


def test_wait_for_new_instance_ignores_stale_status(tmp_path, monkeypatch):
    status_file = tmp_path / "status.json"
    monkeypatch.setattr(app, "STATUS_FILE", status_file)
    monkeypatch.setattr(app, "UPDATE_READY_TIMEOUT", 0.15)
    _write_status(
        status_file, app.APP_VERSION, heartbeat=int(time.time()) - 3600
    )

    assert not _tray_for_wait()._wait_for_new_instance(
        FakeProcess(), app.APP_VERSION, time.time()
    )


def test_wait_for_new_instance_rejects_other_version(tmp_path, monkeypatch):
    status_file = tmp_path / "status.json"
    monkeypatch.setattr(app, "STATUS_FILE", status_file)
    monkeypatch.setattr(app, "UPDATE_READY_TIMEOUT", 0.15)
    _write_status(status_file, "9.9.9")

    assert not _tray_for_wait()._wait_for_new_instance(
        FakeProcess(), app.APP_VERSION, time.time()
    )


def test_wait_for_new_instance_rejects_early_exit(tmp_path, monkeypatch):
    monkeypatch.setattr(app, "STATUS_FILE", tmp_path / "status.json")

    assert not _tray_for_wait()._wait_for_new_instance(
        FakeProcess(returncode=1), app.APP_VERSION, time.time()
    )


def test_update_retry_backoff():
    failures = 0
    delays = []
    for _ in range(6):
        failures, delay = app._next_update_delay(False, failures)
        delays.append(delay)

    assert delays == [60, 300, 900, 3600, 21600, 21600]
    assert failures == 6
    assert app._next_update_delay(True, 5) == (0, app.UPDATE_CHECK_INTERVAL)


def test_wait_for_new_instance_times_out(tmp_path, monkeypatch):
    monkeypatch.setattr(app, "STATUS_FILE", tmp_path / "status.json")
    monkeypatch.setattr(app, "UPDATE_READY_TIMEOUT", 0.15)

    assert not _tray_for_wait()._wait_for_new_instance(
        FakeProcess(), app.APP_VERSION, time.time()
    )


def test_app_version_is_release_formatted():
    assert re.fullmatch(r"\d+\.\d+\.\d+", app.APP_VERSION)


# ---------------------------------------------------------------------------
# Stopping the updated instance and the rollback path
# ---------------------------------------------------------------------------

class _TreeProcess:
    def __init__(self, returncode=None, waits=None):
        self.returncode = returncode
        self.pid = 4242
        self.killed = False
        self._waits = list(waits or [0])

    def poll(self):
        return self.returncode

    def wait(self, timeout=None):
        result = self._waits.pop(0) if self._waits else 0
        if isinstance(result, Exception):
            raise result
        return result

    def kill(self):
        self.killed = True


class _StubbornProcess(_TreeProcess):
    def kill(self):
        raise OSError("access denied")


def test_stop_process_tree_skips_finished_process(monkeypatch):
    calls = []
    monkeypatch.setattr(
        app.subprocess, "run", lambda *args, **kwargs: calls.append(args)
    )

    assert app._stop_process_tree(_TreeProcess(returncode=0))
    assert calls == []


def test_stop_process_tree_uses_taskkill_on_whole_tree(monkeypatch):
    calls = []

    def fake_run(args, **kwargs):
        calls.append((args, kwargs))

    monkeypatch.setattr(app.subprocess, "run", fake_run)

    assert app._stop_process_tree(_TreeProcess())
    assert calls[0][0] == ["taskkill", "/PID", "4242", "/T", "/F"]
    assert calls[0][1]["creationflags"] == app.CREATE_NO_WINDOW


def test_stop_process_tree_falls_back_to_kill(monkeypatch):
    def fake_run(args, **kwargs):
        raise OSError("taskkill is not available")

    monkeypatch.setattr(app.subprocess, "run", fake_run)
    process = _TreeProcess(
        waits=[app.subprocess.TimeoutExpired("taskkill", 5.0), 0]
    )

    assert app._stop_process_tree(process)
    assert process.killed


def test_stop_process_tree_reports_failure(monkeypatch):
    monkeypatch.setattr(app.subprocess, "run", lambda *a, **k: None)
    process = _StubbornProcess(
        waits=[
            app.subprocess.TimeoutExpired("taskkill", 5.0),
            app.subprocess.TimeoutExpired("taskkill", 5.0),
        ]
    )

    assert not app._stop_process_tree(process)


def _tray_with_mutex(handle=None):
    tray = object.__new__(app.BatteryTray)
    tray._mutex = handle
    return tray


def test_reacquire_instance_mutex_keeps_held_handle():
    assert _tray_with_mutex("handle")._reacquire_instance_mutex()


def test_reacquire_instance_mutex_waits_for_the_child(monkeypatch):
    tray = _tray_with_mutex()
    attempts = []

    def acquire(name):
        attempts.append(1)
        return "handle" if len(attempts) >= 2 else None

    monkeypatch.setattr(app, "_acquire_mutex", acquire)
    monkeypatch.setattr(app.time, "sleep", lambda seconds: None)

    assert tray._reacquire_instance_mutex(timeout=1.0)
    assert tray._mutex == "handle"
    assert len(attempts) == 2


def test_reacquire_instance_mutex_gives_up(monkeypatch):
    monkeypatch.setattr(app, "_acquire_mutex", lambda name: None)

    tray = _tray_with_mutex()
    assert not tray._reacquire_instance_mutex(timeout=0.0)
    assert tray._mutex is None


class _FakeProcess:
    returncode = None
    pid = 4242

    def poll(self):
        return None

    def wait(self, timeout=None):
        return 0

    def terminate(self):
        pass

    def kill(self):
        pass


class _FakeIcon:
    def __init__(self):
        self.visible = True
        self.notifications = []
        self.stopped = False

    def notify(self, message, title=None):
        self.notifications.append(message)

    def update_menu(self):
        pass

    def stop(self):
        self.stopped = True


def _tray_for_update(
    tmp_path, monkeypatch, *, healthy, stopped=True, took_over=True,
    restored=True,
):
    target = tmp_path / "LamzuBattery.exe"
    target.write_bytes(b"MZ-old")
    info = updater.UpdateInfo(
        version="1.6.0",
        asset_url="https://github.com/example/release",
        asset_size=10,
        digest="0" * 64,
    )
    calls = {"popen": [], "rollback": 0, "stopped": 0}

    monkeypatch.setattr(updater, "executable_path", lambda: target)
    monkeypatch.setattr(
        updater, "check_for_update", lambda current, **kwargs: info
    )
    monkeypatch.setattr(
        updater, "prepare_update_target", lambda path, timeout=10.0: True
    )
    monkeypatch.setattr(
        updater, "ensure_update_target", lambda path, size=0: None
    )
    monkeypatch.setattr(
        updater,
        "download_update",
        lambda release, dest, progress=None: dest.write_bytes(b"MZ-new"),
    )
    monkeypatch.setattr(updater, "replace_executable", lambda t, n, b: b)

    def rollback(*args):
        calls["rollback"] += 1
        return restored

    monkeypatch.setattr(updater, "rollback_executable", rollback)

    def stop_tree(process, timeout=10.0):
        calls["stopped"] += 1
        return stopped

    monkeypatch.setattr(app, "_stop_process_tree", stop_tree)

    def popen(args, **kwargs):
        calls["popen"].append(args[0])
        return _FakeProcess()

    monkeypatch.setattr(app.subprocess, "Popen", popen)

    tray = object.__new__(app.BatteryTray)
    tray._update_lock = threading.Lock()
    tray._stop = threading.Event()
    tray._auto_update = True
    tray._handed_off = False
    tray._update_pending = False
    tray._available_update = None
    tray._last_offered_version = None
    tray._last_update_error = None
    tray._last_update_error_at = 0.0
    tray._progress_percent = None
    tray._icon = _FakeIcon()
    tray._icon_lock = threading.Lock()
    tray._mutex = None
    tray._write_status = lambda running=True: None
    tray._refresh_menu = lambda: None
    tray._clear_progress = lambda: None
    tray._apply = lambda: None
    tray._release_instance_mutex = lambda: None
    tray._wait_for_new_instance = lambda process, version, started: healthy

    def reacquire(timeout=0.0):
        if took_over and tray._mutex is None:
            tray._mutex = "handle"
        return took_over

    tray._reacquire_instance_mutex = reacquire
    return tray, calls


def test_check_updates_hands_over_to_a_healthy_instance(tmp_path, monkeypatch):
    tray, calls = _tray_for_update(tmp_path, monkeypatch, healthy=True)

    assert tray._check_updates(False)
    assert calls["rollback"] == 0
    assert len(calls["popen"]) == 1
    assert tray._handed_off
    assert tray._icon.stopped


def test_check_updates_rolls_back_and_restarts_the_old_version(
    tmp_path, monkeypatch
):
    tray, calls = _tray_for_update(
        tmp_path, monkeypatch, healthy=False, restored=True
    )

    assert tray._check_updates(False)
    assert calls["stopped"] == 1
    assert calls["rollback"] == 1
    assert len(calls["popen"]) == 2
    assert tray._handed_off
    assert tray._icon.stopped
    assert any(
        "previous version was restored" in note
        for note in tray._icon.notifications
    )


def test_check_updates_keeps_files_when_the_instance_survives(
    tmp_path, monkeypatch
):
    tray, calls = _tray_for_update(
        tmp_path, monkeypatch, healthy=False, took_over=False
    )

    assert tray._check_updates(False)
    assert calls["stopped"] == 1
    assert calls["rollback"] == 0
    assert len(calls["popen"]) == 1
    assert not tray._handed_off
    assert not tray._icon.stopped
    assert any(
        "could not be stopped" in note for note in tray._icon.notifications
    )


def test_check_updates_reports_a_failed_restore(tmp_path, monkeypatch):
    tray, calls = _tray_for_update(
        tmp_path, monkeypatch, healthy=False, restored=False
    )

    assert tray._check_updates(False)
    assert calls["rollback"] == 1
    assert len(calls["popen"]) == 1
    assert not tray._handed_off
    assert any(
        "could not be restored" in note for note in tray._icon.notifications
    )


def test_check_updates_fails_when_preflight_fails(tmp_path, monkeypatch):
    tray, calls = _tray_for_update(tmp_path, monkeypatch, healthy=True)

    def fail(path, size=0):
        raise updater.UpdateError("folder is not writable")

    monkeypatch.setattr(updater, "ensure_update_target", fail)

    assert not tray._check_updates(False)
    assert calls["popen"] == []


def test_check_updates_fails_when_download_fails(tmp_path, monkeypatch):
    tray, calls = _tray_for_update(tmp_path, monkeypatch, healthy=True)

    def fail(release, dest, progress=None):
        raise updater.UpdateError("network is down")

    monkeypatch.setattr(updater, "download_update", fail)

    assert not tray._check_updates(False)
    assert calls["popen"] == []


def test_check_updates_fails_when_replacement_fails(tmp_path, monkeypatch):
    tray, calls = _tray_for_update(tmp_path, monkeypatch, healthy=True)

    def fail(target, new_path, backup_path):
        raise updater.UpdateError("file is locked")

    monkeypatch.setattr(updater, "replace_executable", fail)

    assert not tray._check_updates(False)
    assert not tray._update_pending
    assert calls["popen"] == []


def test_check_updates_fails_when_the_instance_cannot_start(
    tmp_path, monkeypatch
):
    tray, calls = _tray_for_update(tmp_path, monkeypatch, healthy=True)

    def fail(args, **kwargs):
        raise OSError("cannot execute")

    monkeypatch.setattr(app.subprocess, "Popen", fail)

    assert not tray._check_updates(False)
    assert calls["rollback"] == 1
    assert not tray._update_pending
    assert not tray._handed_off


def test_check_updates_treats_a_busy_lock_as_failure(tmp_path, monkeypatch):
    tray, calls = _tray_for_update(tmp_path, monkeypatch, healthy=True)
    assert tray._update_lock.acquire(blocking=False)
    try:
        assert not tray._check_updates(False)
    finally:
        tray._update_lock.release()
    assert calls["popen"] == []


def test_check_updates_restores_the_exe_when_it_disappeared(
    tmp_path, monkeypatch
):
    tray, calls = _tray_for_update(tmp_path, monkeypatch, healthy=True)

    def fail(target, new_path, backup_path):
        target.unlink()
        raise updater.UpdateError("could not install update")

    monkeypatch.setattr(updater, "replace_executable", fail)

    assert not tray._check_updates(False)
    assert calls["rollback"] == 1
    assert any(
        "previous version was restored" in note
        for note in tray._icon.notifications
    )


def test_check_updates_keeps_files_when_the_exe_survived(
    tmp_path, monkeypatch
):
    tray, calls = _tray_for_update(tmp_path, monkeypatch, healthy=True)

    def fail(target, new_path, backup_path):
        raise updater.UpdateError("file is locked")

    monkeypatch.setattr(updater, "replace_executable", fail)

    assert not tray._check_updates(False)
    assert calls["rollback"] == 0
    assert any(
        "file is locked" in note for note in tray._icon.notifications
    )


def test_check_updates_reports_a_failed_rollback_after_start_error(
    tmp_path, monkeypatch
):
    tray, calls = _tray_for_update(
        tmp_path, monkeypatch, healthy=True, restored=False
    )

    def fail(args, **kwargs):
        raise OSError("cannot execute")

    monkeypatch.setattr(app.subprocess, "Popen", fail)

    assert not tray._check_updates(False)
    assert calls["rollback"] == 1
    assert any(
        "could not be restored" in note for note in tray._icon.notifications
    )


def test_rollback_copy_is_removed_after_the_grace_period(
    tmp_path, monkeypatch
):
    tray = object.__new__(app.BatteryTray)
    tray._stop = threading.Event()
    removed = []
    monkeypatch.setattr(app, "BACKUP_GRACE_PERIOD", 0.01)
    monkeypatch.setattr(
        updater, "cleanup_backup", lambda path: removed.append(path)
    )
    executable = tmp_path / "LamzuBattery.exe"

    tray._cleanup_backup_later(executable)

    assert removed == [executable]


def test_rollback_copy_is_kept_when_the_app_stops_first(
    tmp_path, monkeypatch
):
    tray = object.__new__(app.BatteryTray)
    tray._stop = threading.Event()
    tray._stop.set()
    removed = []
    monkeypatch.setattr(app, "BACKUP_GRACE_PERIOD", 10.0)
    monkeypatch.setattr(
        updater, "cleanup_backup", lambda path: removed.append(path)
    )

    tray._cleanup_backup_later(tmp_path / "LamzuBattery.exe")

    assert removed == []
