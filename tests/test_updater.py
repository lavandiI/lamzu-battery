import hashlib
import io
import json
import os
import stat
import time

import pytest

import updater


class FakeResponse:
    def __init__(self, data: bytes, url: str):
        self._data = io.BytesIO(data)
        self._url = url

    def read(self, size: int = -1) -> bytes:
        return self._data.read(size)

    def geturl(self) -> str:
        return self._url

    def __enter__(self):
        return self

    def __exit__(self, *args):
        return False


def _release_payload(
    tag="v1.3.0",
    asset_url="https://github.com/lavandiI/lamzu-battery/releases/download/v1.3.0/LamzuBattery.exe",
    data=b"MZ-test",
    digest=None,
):
    if digest is None:
        digest = hashlib.sha256(data).hexdigest()
    return {
        "tag_name": tag,
        "html_url": "https://github.com/lavandiI/lamzu-battery/releases/tag/v1.3.0",
        "assets": [
            {
                "name": "LamzuBattery.exe",
                "size": len(data),
                "digest": f"sha256:{digest}",
                "browser_download_url": asset_url,
            }
        ],
    }


def test_parse_and_compare_versions():
    assert updater.parse_version("v1.2.1") == (1, 2, 1)
    assert updater.parse_version("LAMZU Battery 1.10.0") == (1, 10, 0)
    assert updater.parse_version("no version") == ()

    assert updater.is_newer("v1.2.2", "1.2.1")
    assert updater.is_newer("v1.10.0", "1.9.9")
    assert updater.is_newer("v2.0", "1.99.99")
    assert not updater.is_newer("v1.2.1", "1.2.1")
    assert not updater.is_newer("v1.2.0", "1.2.1")
    assert not updater.is_newer("garbage", "1.2.1")


def test_same_version_normalizes_release_form():
    assert updater.same_version("v1.6", "1.6.0")
    assert updater.same_version("1.6.0.0", "1.6")
    assert updater.same_version("1.5.10", "v1.5.10")
    assert not updater.same_version("1.6.1", "1.6.0")
    assert not updater.same_version("", "1.6.0")
    assert not updater.same_version("garbage", "1.6.0")


def test_update_paths_use_actual_exe_name(tmp_path):
    target = tmp_path / "Custom Name.exe"

    new_path, backup_path = updater.update_paths(target)

    assert new_path == tmp_path / "Custom Name.new.exe"
    assert backup_path == tmp_path / "Custom Name.old.exe"


def test_executable_path_is_disabled_from_source():
    assert updater.executable_path() is None


def test_clean_environment_strips_pyinstaller_vars(monkeypatch):
    monkeypatch.setenv("_PYI_APPLICATION_HOME_DIR", "C:/temp/old")
    monkeypatch.setenv("_PYI_ARCHIVE_FILE", "C:/temp/old/app.exe")
    monkeypatch.setenv("LAMZU_KEEP_ME", "1")

    env = updater.clean_environment()

    assert env["LAMZU_KEEP_ME"] == "1"
    assert not [key for key in env if key.upper().startswith("_PYI_")]


def test_check_for_update_parses_release(monkeypatch):
    payload = _release_payload(data=b"MZ-new")
    monkeypatch.setattr(
        updater,
        "_open_url",
        lambda url, timeout, accept: FakeResponse(
            json.dumps(payload).encode("utf-8"), url
        ),
    )

    info = updater.check_for_update("1.2.1")

    assert info is not None
    assert info.version == "1.3.0"
    assert info.asset_size == len(b"MZ-new")
    assert info.digest == hashlib.sha256(b"MZ-new").hexdigest()


def test_check_for_update_ignores_same_version(monkeypatch):
    payload = _release_payload(tag="v1.3.0")
    monkeypatch.setattr(
        updater,
        "_open_url",
        lambda url, timeout, accept: FakeResponse(
            json.dumps(payload).encode("utf-8"), url
        ),
    )

    assert updater.check_for_update("1.3.0") is None


def test_check_for_update_requires_digest(monkeypatch):
    payload = _release_payload()
    del payload["assets"][0]["digest"]
    monkeypatch.setattr(
        updater,
        "_open_url",
        lambda url, timeout, accept: FakeResponse(
            json.dumps(payload).encode("utf-8"), url
        ),
    )

    with pytest.raises(updater.UpdateError):
        updater.check_for_update("1.2.1")


def test_check_for_update_accepts_any_repository_case(monkeypatch):
    payload = _release_payload(
        asset_url=(
            "https://github.com/Lavandii/Lamzu-Battery/releases/download/"
            "v1.3.0/LamzuBattery.exe"
        )
    )
    monkeypatch.setattr(
        updater,
        "_open_url",
        lambda url, timeout, accept: FakeResponse(
            json.dumps(payload).encode("utf-8"), url
        ),
    )

    assert updater.check_for_update("1.2.1") is not None


def test_check_for_update_rejects_untrusted_url(monkeypatch):
    payload = _release_payload(
        asset_url="https://example.com/LamzuBattery.exe"
    )
    monkeypatch.setattr(
        updater,
        "_open_url",
        lambda url, timeout, accept: FakeResponse(
            json.dumps(payload).encode("utf-8"), url
        ),
    )

    with pytest.raises(updater.UpdateError):
        updater.check_for_update("1.2.1")


def test_download_update_saves_verified_file(monkeypatch, tmp_path):
    data = b"MZ" + b"payload"
    payload = _release_payload(data=data)
    monkeypatch.setattr(
        updater,
        "_open_url",
        lambda url, timeout, accept: FakeResponse(
            json.dumps(payload).encode("utf-8"), url
        ),
    )
    info = updater.check_for_update("1.2.1")

    def fake_open(url, timeout, accept):
        return FakeResponse(data, url)

    monkeypatch.setattr(updater, "_open_url", fake_open)
    destination = tmp_path / "LamzuBattery.new.exe"
    progress = []

    updater.download_update(
        info,
        destination,
        progress=lambda done, total: progress.append((done, total)),
    )

    assert destination.read_bytes() == data
    assert progress[0] == (0, len(data))
    assert progress[-1] == (len(data), len(data))


def test_ensure_update_target_succeeds(tmp_path):
    target = tmp_path / "LamzuBattery.exe"
    target.write_bytes(b"current")

    updater.ensure_update_target(target, required_bytes=1024)


def test_ensure_update_target_reports_missing_space(tmp_path):
    target = tmp_path / "LamzuBattery.exe"
    target.write_bytes(b"current")

    with pytest.raises(updater.UpdateError, match="free space"):
        updater.ensure_update_target(target, required_bytes=10**15)


def test_download_update_rejects_bad_checksum(monkeypatch, tmp_path):
    data = b"MZ" + b"payload"
    payload = _release_payload(data=data)
    monkeypatch.setattr(
        updater,
        "_open_url",
        lambda url, timeout, accept: FakeResponse(
            json.dumps(payload).encode("utf-8"), url
        ),
    )
    info = updater.check_for_update("1.2.1")

    bad_data = b"MZ" + b"tampered"
    monkeypatch.setattr(
        updater,
        "_open_url",
        lambda url, timeout, accept: FakeResponse(bad_data, url),
    )
    destination = tmp_path / "LamzuBattery.new.exe"

    with pytest.raises(updater.UpdateError):
        updater.download_update(info, destination)

    assert not destination.exists()


def test_replace_executable_keeps_backup(tmp_path):
    target = tmp_path / "LamzuBattery.exe"
    new_path = tmp_path / "LamzuBattery.new.exe"
    target.write_bytes(b"old")
    new_path.write_bytes(b"new")

    backup = updater.replace_executable(target, new_path)

    assert target.read_bytes() == b"new"
    assert backup.read_bytes() == b"old"
    assert not new_path.exists()


def test_replace_executable_refuses_to_overwrite_backup(tmp_path):
    target = tmp_path / "LamzuBattery.exe"
    new_path = tmp_path / "LamzuBattery.new.exe"
    backup_path = tmp_path / "LamzuBattery.old.exe"
    target.write_bytes(b"current")
    new_path.write_bytes(b"new")
    backup_path.write_bytes(b"previous")

    with pytest.raises(updater.UpdateError):
        updater.replace_executable(target, new_path, backup_path)

    assert target.read_bytes() == b"current"
    assert backup_path.read_bytes() == b"previous"


def test_cleanup_download_files_keeps_backup(tmp_path):
    target = tmp_path / "LamzuBattery.exe"
    target.write_bytes(b"current")
    new_path, backup_path = updater.update_paths(target)
    new_path.write_bytes(b"new")
    backup_path.write_bytes(b"old")
    part_path = new_path.with_name(new_path.name + ".part")
    part_path.write_bytes(b"partial")

    updater.cleanup_download_files(target, attempts=1)

    assert not new_path.exists()
    assert not part_path.exists()
    assert backup_path.read_bytes() == b"old"
    assert target.read_bytes() == b"current"


def test_replace_with_retries_handles_busy_file(monkeypatch, tmp_path):
    source = tmp_path / "source.exe"
    destination = tmp_path / "destination.exe"
    source.write_bytes(b"MZ-new")
    calls = []
    real_replace = updater.os.replace

    def flaky_replace(src, dst):
        calls.append((src, dst))
        if len(calls) < 3:
            raise PermissionError("file is busy")
        return real_replace(src, dst)

    monkeypatch.setattr(updater.os, "replace", flaky_replace)

    updater._replace_with_retries(source, destination, attempts=3)

    assert destination.read_bytes() == b"MZ-new"
    assert len(calls) == 3


def test_cleanup_stale_backup_removes_only_old_copy(tmp_path):
    target = tmp_path / "LamzuBattery.exe"
    target.write_bytes(b"current")
    _, backup = updater.update_paths(target)
    backup.write_bytes(b"old")
    old_time = time.time() - updater.STALE_BACKUP_SECONDS - 60
    os.utime(backup, (old_time, old_time))

    assert updater.cleanup_stale_backup(target)
    assert not backup.exists()


def test_cleanup_stale_backup_keeps_fresh_copy(tmp_path):
    target = tmp_path / "LamzuBattery.exe"
    target.write_bytes(b"current")
    _, backup = updater.update_paths(target)
    backup.write_bytes(b"old")

    assert not updater.cleanup_stale_backup(target)
    assert backup.exists()


def test_prepare_update_target_removes_stale_backup(tmp_path):
    target = tmp_path / "LamzuBattery.exe"
    target.write_bytes(b"current")
    _, backup = updater.update_paths(target)
    backup.write_bytes(b"old")
    old_time = time.time() - updater.STALE_BACKUP_SECONDS - 60
    os.utime(backup, (old_time, old_time))

    assert updater.prepare_update_target(target, timeout=0.1)
    assert not backup.exists()


def test_prepare_update_target_removes_a_fresh_backup(tmp_path):
    # The copy is kept for a while after startup, so a new update has to
    # clear it even when it is fresh.
    target = tmp_path / "LamzuBattery.exe"
    target.write_bytes(b"current")
    _, backup = updater.update_paths(target)
    backup.write_bytes(b"old")

    assert updater.prepare_update_target(target, timeout=0.1)
    assert not backup.exists()


def test_rollback_executable_swaps_the_files_back(tmp_path):
    target = tmp_path / "LamzuBattery.exe"
    new_path, backup = updater.update_paths(target)
    target.write_bytes(b"new")
    backup.write_bytes(b"old")

    assert updater.rollback_executable(target, backup, new_path)
    assert target.read_bytes() == b"old"
    assert new_path.read_bytes() == b"new"
    assert not backup.exists()


def test_rollback_executable_restores_a_missing_target(tmp_path):
    # A failed replacement can leave no exe at all: the backup is then the
    # only copy of the previous version and has to be restored directly.
    target = tmp_path / "LamzuBattery.exe"
    new_path, backup = updater.update_paths(target)
    backup.write_bytes(b"old")
    new_path.write_bytes(b"new")

    assert updater.rollback_executable(target, backup, new_path)
    assert target.read_bytes() == b"old"
    assert not backup.exists()
    assert new_path.read_bytes() == b"new"


def test_rollback_executable_reports_a_missing_backup(tmp_path):
    target = tmp_path / "LamzuBattery.exe"
    new_path, backup = updater.update_paths(target)
    target.write_bytes(b"new")

    assert not updater.rollback_executable(target, backup, new_path)
    assert target.read_bytes() == b"new"


def test_cleanup_backup_keeps_download(tmp_path):
    target = tmp_path / "LamzuBattery.exe"
    target.write_bytes(b"current")
    new_path, backup_path = updater.update_paths(target)
    new_path.write_bytes(b"new")
    backup_path.write_bytes(b"old")

    updater.cleanup_backup(target, attempts=1)

    assert not backup_path.exists()
    assert new_path.read_bytes() == b"new"
    assert target.read_bytes() == b"current"


def test_replace_executable_marks_the_backup_as_new(tmp_path):
    target = tmp_path / "LamzuBattery.exe"
    new_path = tmp_path / "LamzuBattery.new.exe"
    target.write_bytes(b"old")
    new_path.write_bytes(b"new")
    week_ago = time.time() - 7 * 24 * 60 * 60
    os.utime(target, (week_ago, week_ago))

    backup = updater.replace_executable(target, new_path)

    # The rename keeps the source mtime; the copy must look fresh instead.
    assert abs(time.time() - backup.stat().st_mtime) < 60


def test_ensure_update_target_rejects_a_read_only_exe(tmp_path):
    target = tmp_path / "LamzuBattery.exe"
    target.write_bytes(b"current")
    os.chmod(target, stat.S_IREAD)
    try:
        with pytest.raises(updater.UpdateError, match="read-only"):
            updater.ensure_update_target(target)
    finally:
        os.chmod(target, stat.S_IWRITE)


def test_download_update_times_out(monkeypatch, tmp_path):
    data = b"MZ" + b"payload"
    payload = _release_payload(data=data)
    monkeypatch.setattr(
        updater,
        "_open_url",
        lambda url, timeout, accept: FakeResponse(
            json.dumps(payload).encode("utf-8"), url
        ),
    )
    info = updater.check_for_update("1.2.1")
    monkeypatch.setattr(
        updater,
        "_open_url",
        lambda url, timeout, accept: FakeResponse(data, url),
    )

    clock = [0.0]
    monkeypatch.setattr(
        updater.time,
        "monotonic",
        lambda: clock.__setitem__(0, clock[0] + 10 ** 6) or clock[0],
    )

    destination = tmp_path / "LamzuBattery.new.exe"
    with pytest.raises(updater.UpdateError, match="timed out"):
        updater.download_update(info, destination)

    assert not destination.exists()
    assert not (tmp_path / "LamzuBattery.new.exe.part").exists()
