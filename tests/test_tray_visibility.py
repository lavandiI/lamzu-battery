import os

import tray_visibility as tv


def _supported(monkeypatch):
    monkeypatch.setattr(tv, "is_supported", lambda: True)


def test_missing_value_is_promoted(monkeypatch):
    _supported(monkeypatch)
    calls = []
    monkeypatch.setattr(
        tv, "_entries_for", lambda path: [tv.TrayEntry("entry", None)]
    )
    monkeypatch.setattr(
        tv, "_set_promoted", lambda key: calls.append(key)
    )

    assert tv.promote_if_unconfigured("C:/Lamzu.exe", retry_delays=()) == (
        tv.PROMOTED
    )
    assert calls == ["entry"]


def test_explicit_zero_is_respected(monkeypatch):
    _supported(monkeypatch)
    calls = []
    monkeypatch.setattr(
        tv, "_entries_for", lambda path: [tv.TrayEntry("entry", 0)]
    )
    monkeypatch.setattr(tv, "_set_promoted", lambda key: calls.append(key))

    assert tv.promote_if_unconfigured("C:/Lamzu.exe", retry_delays=()) == (
        tv.OPTED_OUT
    )
    assert calls == []


def test_already_promoted_is_noop(monkeypatch):
    _supported(monkeypatch)
    calls = []
    monkeypatch.setattr(
        tv, "_entries_for", lambda path: [tv.TrayEntry("entry", 1)]
    )
    monkeypatch.setattr(tv, "_set_promoted", lambda key: calls.append(key))

    assert tv.promote_if_unconfigured("C:/Lamzu.exe", retry_delays=()) == (
        tv.ALREADY
    )
    assert calls == []


def test_missing_registry_entry_is_not_found(monkeypatch):
    _supported(monkeypatch)
    monkeypatch.setattr(tv, "_entries_for", lambda path: [])

    assert tv.promote_if_unconfigured("C:/Lamzu.exe", retry_delays=()) == (
        tv.NOT_FOUND
    )


def test_unsupported_windows_is_noop(monkeypatch):
    monkeypatch.setattr(tv, "is_supported", lambda: False)
    monkeypatch.setattr(
        tv, "_entries_for", lambda path: [tv.TrayEntry("entry", None)]
    )

    assert tv.promote_if_unconfigured("C:/Lamzu.exe", retry_delays=()) == (
        tv.UNSUPPORTED
    )


def test_manual_promotion_overrides_zero(monkeypatch):
    _supported(monkeypatch)
    calls = []
    monkeypatch.setattr(
        tv, "_entries_for", lambda path: [tv.TrayEntry("entry", 0)]
    )
    monkeypatch.setattr(tv, "_set_promoted", lambda key: calls.append(key))

    assert tv.promote("C:/Lamzu.exe", retry_delays=()) == tv.PROMOTED
    assert calls == ["entry"]


def test_zero_wins_over_duplicate_entries(monkeypatch):
    _supported(monkeypatch)
    calls = []
    monkeypatch.setattr(
        tv,
        "_entries_for",
        lambda path: [
            tv.TrayEntry("hidden", 0),
            tv.TrayEntry("new", None),
        ],
    )
    monkeypatch.setattr(tv, "_set_promoted", lambda key: calls.append(key))

    assert tv.promote_if_unconfigured("C:/Lamzu.exe", retry_delays=()) == (
        tv.OPTED_OUT
    )
    assert calls == []


def test_normalize_path_is_case_insensitive_and_strips_quotes():
    expected = os.path.normcase(os.path.normpath(r"C:\Tools\Lamzu.exe"))
    assert tv._normalize(' "C:\\Tools\\Lamzu.exe" ') == expected


def test_entries_survive_an_enumeration_error(monkeypatch):
    exe = r"C:\Tools\Lamzu.exe"

    class _Key:
        def __init__(self, name):
            self.name = name

        def __enter__(self):
            return self

        def __exit__(self, *args):
            return False

    def open_key(root, name, *args, **kwargs):
        if name == tv.NOTIFY_ICON_KEY:
            return _Key("root")
        if name == "ok":
            return _Key("ok")
        raise OSError("unreadable key")

    def enum_key(root, index):
        if index == 0:
            return "ok"
        raise OSError("enumeration failed")

    def query_value(key, name):
        if key.name == "ok" and name == tv.EXECUTABLE_PATH:
            return (exe, 1)
        raise FileNotFoundError()

    monkeypatch.setattr(tv.winreg, "OpenKey", open_key)
    monkeypatch.setattr(tv.winreg, "QueryInfoKey", lambda key: (2, 0, 0))
    monkeypatch.setattr(tv.winreg, "EnumKey", enum_key)
    monkeypatch.setattr(tv.winreg, "QueryValueEx", query_value)

    entries = tv._entries_for(exe)

    assert [entry.key_name for entry in entries] == ["ok"]


def test_manual_promotion_retries_after_a_write_error(monkeypatch):
    _supported(monkeypatch)
    calls = []
    failures = [OSError("busy")]

    def set_promoted(key):
        if failures:
            raise failures.pop(0)
        calls.append(key)

    monkeypatch.setattr(
        tv, "_entries_for", lambda path: [tv.TrayEntry("entry", 0)]
    )
    monkeypatch.setattr(tv, "_set_promoted", set_promoted)
    monkeypatch.setattr(tv, "_wait", lambda delay, stop: False)

    assert tv.promote("C:/Lamzu.exe", retry_delays=(0.0,)) == tv.PROMOTED
    assert calls == ["entry"]


def test_manual_promotion_reports_a_persistent_error(monkeypatch):
    _supported(monkeypatch)

    def boom(key):
        raise OSError("denied")

    monkeypatch.setattr(
        tv, "_entries_for", lambda path: [tv.TrayEntry("entry", None)]
    )
    monkeypatch.setattr(tv, "_set_promoted", boom)
    monkeypatch.setattr(tv, "_wait", lambda delay, stop: False)

    assert tv.promote("C:/Lamzu.exe", retry_delays=(0.0,)) == tv.ERROR
