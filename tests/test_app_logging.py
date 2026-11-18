import logging
import sys
from logging.handlers import RotatingFileHandler
from pathlib import Path

import app

_NOISY = ("PIL", "pystray", "hid")


def _snapshot():
    root = logging.getLogger()
    return (
        root.level,
        list(root.handlers),
        {name: logging.getLogger(name).level for name in _NOISY},
    )


def _restore(snapshot):
    level, handlers, noisy = snapshot
    root = logging.getLogger()
    root.setLevel(level)
    for handler in list(root.handlers):
        if handler not in handlers:
            root.removeHandler(handler)
            handler.close()
    for name, value in noisy.items():
        logging.getLogger(name).setLevel(value)


def test_setup_logging_writes_to_the_data_folder(tmp_path, monkeypatch):
    monkeypatch.setattr(app, "DATA_DIR", tmp_path)
    monkeypatch.setattr(app, "LOG_FILE", tmp_path / "app.log")
    snapshot = _snapshot()
    try:
        assert app.setup_logging()
        assert (tmp_path / "app.log").exists()
    finally:
        _restore(snapshot)


def test_setup_logging_falls_back_to_temp(tmp_path, monkeypatch):
    blocker = tmp_path / "file"
    blocker.write_text("x", encoding="ascii")
    temp = tmp_path / "temp"
    temp.mkdir()
    monkeypatch.setattr(app, "DATA_DIR", blocker / "data")
    monkeypatch.setattr(app, "LOG_FILE", blocker / "data" / "app.log")
    monkeypatch.setattr(app.tempfile, "gettempdir", lambda: str(temp))
    snapshot = _snapshot()
    try:
        assert app.setup_logging()
        assert (temp / f"{app.APP_ID}.log").exists()
    finally:
        _restore(snapshot)


def test_setup_logging_does_not_crash_when_nothing_is_writable(
    tmp_path, monkeypatch
):
    blocker = tmp_path / "file"
    blocker.write_text("x", encoding="ascii")
    monkeypatch.setattr(app, "DATA_DIR", blocker / "data")
    monkeypatch.setattr(app, "LOG_FILE", blocker / "data" / "app.log")
    monkeypatch.setattr(
        app.tempfile, "gettempdir", lambda: str(tmp_path / "missing")
    )
    snapshot = _snapshot()
    try:
        assert app.setup_logging() is False
    finally:
        _restore(snapshot)


def test_setup_logging_does_not_duplicate_the_handler(tmp_path, monkeypatch):
    monkeypatch.setattr(app, "DATA_DIR", tmp_path)
    monkeypatch.setattr(app, "LOG_FILE", tmp_path / "app.log")
    snapshot = _snapshot()
    try:
        assert app.setup_logging()
        assert app.setup_logging()
        handlers = [
            handler
            for handler in logging.getLogger().handlers
            if isinstance(handler, RotatingFileHandler)
            and Path(handler.baseFilename) == tmp_path / "app.log"
        ]
        assert len(handlers) == 1
    finally:
        _restore(snapshot)


def test_install_exception_logging_survives_a_failing_faulthandler(
    tmp_path, monkeypatch
):
    monkeypatch.setattr(app, "CRASH_FILE", tmp_path / "crash.log")

    def boom(**kwargs):
        raise RuntimeError("no fault handler")

    monkeypatch.setattr(app.faulthandler, "enable", boom)
    try:
        app.install_exception_logging()
        assert app._FAULT_FILE is None
        assert sys.excepthook is app._unhandled_exception
    finally:
        app.shutdown_exception_logging()


def test_install_exception_logging_survives_a_bad_crash_file(
    tmp_path, monkeypatch
):
    blocker = tmp_path / "file"
    blocker.write_text("x", encoding="ascii")
    monkeypatch.setattr(app, "CRASH_FILE", blocker / "crash.log")
    try:
        app.install_exception_logging()
        assert sys.excepthook is app._unhandled_exception
        assert app._FAULT_FILE is None
    finally:
        app.shutdown_exception_logging()
