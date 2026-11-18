import json
from pathlib import Path

import app


def _use_tmp_settings(tmp_path, monkeypatch):
    monkeypatch.setattr(app, "DATA_DIR", tmp_path)
    monkeypatch.setattr(app, "SETTINGS_FILE", tmp_path / "settings.json")


def test_settings_are_merged_not_replaced(tmp_path, monkeypatch):
    _use_tmp_settings(tmp_path, monkeypatch)

    app._save_settings(language="ru")
    app._save_settings(update_mode="notify")

    data = json.loads((tmp_path / "settings.json").read_text(encoding="utf-8"))

    assert data["language"] == "ru"
    assert data["update_mode"] == "notify"


def test_unknown_update_mode_falls_back_to_auto(tmp_path, monkeypatch):
    _use_tmp_settings(tmp_path, monkeypatch)
    (tmp_path / "settings.json").write_text(
        json.dumps({"language": "en", "update_mode": "sometimes"}),
        encoding="utf-8",
    )

    settings = app._load_settings()

    assert settings["language"] == "en"
    assert settings["update_mode"] == "auto"


def test_save_settings_keeps_unknown_keys(tmp_path, monkeypatch):
    _use_tmp_settings(tmp_path, monkeypatch)
    (tmp_path / "settings.json").write_text(
        json.dumps({"future_key": "keep me"}),
        encoding="utf-8",
    )

    app._save_settings(language="en")

    data = json.loads((tmp_path / "settings.json").read_text(encoding="utf-8"))
    assert data["future_key"] == "keep me"
    assert data["language"] == "en"


def test_save_settings_leaves_no_temp_file(tmp_path, monkeypatch):
    _use_tmp_settings(tmp_path, monkeypatch)

    app._save_settings(language="ru")

    leftovers = [path.name for path in tmp_path.iterdir() if path.suffix == ".tmp"]
    assert leftovers == []


def test_migrate_legacy_autostart_survives_registry_errors(monkeypatch):
    def boom(*args, **kwargs):
        raise OSError("access denied")

    monkeypatch.setattr(app.winreg, "OpenKey", boom)

    app._migrate_legacy_autostart()


class _Key:
    def __enter__(self):
        return self

    def __exit__(self, *args):
        return False


def test_autostart_is_detected_regardless_of_case(monkeypatch):
    command = app.autostart_command()
    monkeypatch.setattr(app.winreg, "OpenKey", lambda *args, **kwargs: _Key())
    monkeypatch.setattr(
        app.winreg, "QueryValueEx", lambda key, name: (command.upper(), 1)
    )

    assert app.autostart_enabled()


def test_autostart_ignores_a_different_command(monkeypatch):
    monkeypatch.setattr(app.winreg, "OpenKey", lambda *args, **kwargs: _Key())
    monkeypatch.setattr(
        app.winreg,
        "QueryValueEx",
        lambda key, name: ('"C:\\Other\\Lamzu.exe"', 1),
    )

    assert not app.autostart_enabled()


def test_settings_with_bom_are_not_reset(tmp_path, monkeypatch):
    # Notepad used to save UTF-8 with a BOM; the file must still be read.
    _use_tmp_settings(tmp_path, monkeypatch)
    (tmp_path / "settings.json").write_text(
        json.dumps({"language": "ru", "update_mode": "notify"}),
        encoding="utf-8-sig",
    )

    settings = app._load_settings()

    assert settings["language"] == "ru"
    assert settings["update_mode"] == "notify"


def test_writable_dir_leaves_no_probe_file(tmp_path):
    assert app._is_writable_dir(tmp_path)
    assert list(tmp_path.iterdir()) == []


def test_writable_dir_reports_failure(tmp_path):
    blocker = tmp_path / "file"
    blocker.write_text("x", encoding="ascii")

    assert not app._is_writable_dir(blocker / "data")


def test_data_dir_falls_back_when_the_app_folder_is_read_only(
    tmp_path, monkeypatch
):
    preferred = tmp_path / "app"
    local = tmp_path / "local"
    monkeypatch.setattr(app, "BASE_DIR", preferred)
    monkeypatch.setenv("LOCALAPPDATA", str(local))
    monkeypatch.setattr(app, "_DATA_DIR_FALLBACK", None)
    real = app._is_writable_dir
    monkeypatch.setattr(
        app,
        "_is_writable_dir",
        lambda path: path != preferred / "data" and real(path),
    )

    assert app._pick_data_dir() == local / app.APP_ID
    assert app._DATA_DIR_FALLBACK == preferred / "data"


def test_data_dir_uses_home_when_localappdata_is_empty(
    tmp_path, monkeypatch
):
    preferred = tmp_path / "app"
    monkeypatch.setattr(app, "BASE_DIR", preferred)
    monkeypatch.setenv("LOCALAPPDATA", "")
    monkeypatch.setattr(app, "_DATA_DIR_FALLBACK", None)
    monkeypatch.setattr(
        app, "_is_writable_dir", lambda path: path != preferred / "data"
    )

    assert app._pick_data_dir() == Path.home() / app.APP_ID


def test_data_dir_keeps_preferred_when_nothing_is_writable(
    tmp_path, monkeypatch
):
    preferred = tmp_path / "app"
    monkeypatch.setattr(app, "BASE_DIR", preferred)
    monkeypatch.setattr(app, "_DATA_DIR_FALLBACK", None)
    monkeypatch.setattr(app, "_is_writable_dir", lambda path: False)

    assert app._pick_data_dir() == preferred / "data"
    assert app._DATA_DIR_FALLBACK == preferred / "data"
