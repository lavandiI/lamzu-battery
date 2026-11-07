import logging
import sys
from types import SimpleNamespace

import app


def _exception():
    try:
        raise RuntimeError("boom")
    except RuntimeError:
        return sys.exc_info()


def test_main_thread_exception_is_logged(monkeypatch, caplog):
    exc_type, exc_value, exc_traceback = _exception()
    messages = []
    monkeypatch.setattr(
        app, "_show_crash_message", lambda: messages.append(True)
    )

    with caplog.at_level(logging.CRITICAL, logger="lamzu_tray"):
        app._unhandled_exception(exc_type, exc_value, exc_traceback)

    assert "unhandled exception" in caplog.text
    assert "RuntimeError: boom" in caplog.text
    assert messages == [True]


def test_system_exit_is_not_reported(caplog):
    with caplog.at_level(logging.CRITICAL, logger="lamzu_tray"):
        app._unhandled_exception(SystemExit, SystemExit(1), None)

    assert "unhandled exception" not in caplog.text


def test_thread_exception_is_logged(caplog):
    exc_type, exc_value, exc_traceback = _exception()
    args = SimpleNamespace(
        exc_type=exc_type,
        exc_value=exc_value,
        exc_traceback=exc_traceback,
        thread=SimpleNamespace(name="worker"),
    )

    with caplog.at_level(logging.CRITICAL, logger="lamzu_tray"):
        app._unhandled_thread(args)

    assert "unhandled exception in thread worker" in caplog.text
    assert "RuntimeError: boom" in caplog.text


def test_install_and_shutdown_exception_logging(tmp_path, monkeypatch):
    monkeypatch.setattr(app, "CRASH_FILE", tmp_path / "crash.log")
    try:
        app.install_exception_logging()
        assert sys.excepthook is app._unhandled_exception
        assert (tmp_path / "crash.log").exists()
    finally:
        app.shutdown_exception_logging()

    assert sys.excepthook is sys.__excepthook__
