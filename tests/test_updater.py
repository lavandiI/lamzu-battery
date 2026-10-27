import hashlib
import io
import json

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


def test_update_paths_use_actual_exe_name(tmp_path):
    target = tmp_path / "Custom Name.exe"

    new_path, backup_path = updater.update_paths(target)

    assert new_path == tmp_path / "Custom Name.new.exe"
    assert backup_path == tmp_path / "Custom Name.old.exe"


def test_executable_path_is_disabled_from_source():
    assert updater.executable_path() is None


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
    assert info.tag == "v1.3.0"
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

    updater.download_update(info, destination)

    assert destination.read_bytes() == data


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


def test_cleanup_update_files(tmp_path):
    target = tmp_path / "LamzuBattery.exe"
    target.write_bytes(b"current")
    new_path, backup_path = updater.update_paths(target)
    new_path.write_bytes(b"new")
    backup_path.write_bytes(b"old")
    part_path = new_path.with_name(new_path.name + ".part")
    part_path.write_bytes(b"partial")

    updater.cleanup_update_files(target, attempts=1)

    assert not new_path.exists()
    assert not backup_path.exists()
    assert not part_path.exists()
    assert target.read_bytes() == b"current"
