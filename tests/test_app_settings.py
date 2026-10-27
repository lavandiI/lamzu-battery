import json

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
