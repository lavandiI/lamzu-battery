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
