import sys

import pytest

import diagnose
import i18n
import lamzu
from lamzu import BatteryReading, DeviceInfo


@pytest.fixture(autouse=True)
def english():
    previous = i18n.get_language()
    i18n.set_language("en")
    yield
    i18n.set_language(previous)


def _info(**overrides):
    info = {
        "vendor_id": 0x3554,
        "product_id": 0xF50F,
        "path": b"path-1",
        "product_string": "LAMZU THORN",
        "manufacturer_string": "LAMZU",
        "usage_page": 0xFF02,
        "interface_number": 1,
    }
    info.update(overrides)
    return info


def _control():
    return DeviceInfo(
        path=b"path-1",
        vendor_id=0x3554,
        product_id=0xF50F,
        product="LAMZU THORN",
        manufacturer="LAMZU",
        usage_page=0xFF02,
        interface=1,
        protocol="compx",
        model="Thorn",
    )


def test_print_devices_marks_the_control_interface(capsys):
    found = diagnose._print_devices([_info()], {b"path-1": _control()})

    out = capsys.readouterr().out
    assert found == 1
    assert "3554:F50F" in out
    assert "control: compx" in out
    assert "LAMZU THORN" in out


def test_print_devices_skips_other_vendors(capsys):
    found = diagnose._print_devices(
        [
            _info(
                vendor_id=0x1234,
                product_string="Keyboard",
                manufacturer_string="ACME",
            )
        ],
        {},
    )

    assert found == 0
    assert capsys.readouterr().out == ""


def test_print_raw_prints_hex(monkeypatch, capsys):
    monkeypatch.setattr(
        lamzu, "probe_reports", lambda device: [bytes([0x08, 0x04, 0x5F])]
    )

    diagnose._print_raw({b"path-1": _control()})

    assert "08 04 5f" in capsys.readouterr().out


def test_print_raw_reports_no_data(monkeypatch, capsys):
    monkeypatch.setattr(lamzu, "probe_reports", lambda device: [])

    diagnose._print_raw({b"path-1": _control()})

    assert "no report received" in capsys.readouterr().out


def test_main_returns_one_without_a_device(monkeypatch, capsys):
    monkeypatch.setattr(diagnose.hid, "enumerate", lambda: [])
    monkeypatch.setattr(diagnose, "read_battery", lambda: None)
    monkeypatch.setattr(sys, "argv", ["diagnose.py", "--lang", "en"])

    assert diagnose.main() == 1
    assert "not found" in capsys.readouterr().out


def test_main_returns_zero_when_the_mouse_answers(monkeypatch, capsys):
    monkeypatch.setattr(diagnose.hid, "enumerate", lambda: [_info()])
    monkeypatch.setattr(
        diagnose,
        "read_battery",
        lambda: BatteryReading(
            percent=99,
            state=0,
            model="Thorn",
            protocol="compx",
            voltage_mv=4100,
            raw_percent=95,
        ),
    )
    monkeypatch.setattr(sys, "argv", ["diagnose.py", "--lang", "en"])

    assert diagnose.main() == 0
    out = capsys.readouterr().out
    assert "Battery: 99%" in out
    assert "level byte: 95%" in out


def test_main_raw_mode_prints_reports(monkeypatch, capsys):
    monkeypatch.setattr(diagnose.hid, "enumerate", lambda: [_info()])
    monkeypatch.setattr(
        lamzu, "probe_reports", lambda device: [bytes([0x08, 0x04, 0x00])]
    )
    monkeypatch.setattr(diagnose, "read_battery", lambda: None)
    monkeypatch.setattr(
        sys, "argv", ["diagnose.py", "--lang", "en", "--raw"]
    )

    assert diagnose.main() == 1
    out = capsys.readouterr().out
    assert "Raw reports:" in out
    assert "08 04 00" in out
