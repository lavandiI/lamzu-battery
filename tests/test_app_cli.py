import sys

import pytest

import app
import i18n


def test_version_flag_prints_the_version(monkeypatch, capsys):
    monkeypatch.setattr(sys, "argv", ["app.py", "--version"])

    with pytest.raises(SystemExit) as exit_info:
        app.main()

    assert exit_info.value.code == 0
    assert app.APP_VERSION in capsys.readouterr().out


def test_language_menu_lists_every_language():
    tray = object.__new__(app.BatteryTray)

    menu = tray._language_menu()

    assert [item.text for item in menu.items] == [
        i18n.LANGUAGE_NAMES[code] for code in i18n.LANGUAGES
    ]
    assert all(item.radio for item in menu.items)
