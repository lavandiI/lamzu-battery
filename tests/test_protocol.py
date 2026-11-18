"""Protocol parsing and request layout tests.

These tests never touch real hardware: they build synthetic HID reports and
check the parsers used by the tray application.
"""

import pytest

import lamzu


def _compx_report(level=85, state=0, voltage_mv=4046):
    """Real 17-byte frame from a 1K dongle: 08 04 00 00 00 02 .. .. .. sum."""
    report = [0] * 17
    report[0] = 0x08
    report[1] = 0x04
    report[5] = 0x02
    report[6] = level
    report[7] = state
    report[8] = (voltage_mv >> 8) & 0xFF
    report[9] = voltage_mv & 0xFF
    report[16] = (0x55 - sum(report[:16])) & 0xFF
    return report


def _compx_report_without_id(level=55, state=0, voltage_mv=4046):
    """The same frame with the report ID stripped (16 bytes, shifted by one)."""
    frame = [
        0x04,
        0x00,
        0x00,
        0x00,
        0x02,
        level,
        state,
        (voltage_mv >> 8) & 0xFF,
        voltage_mv & 0xFF,
    ] + [0] * 6
    frame.append((0x55 - 0x08 - sum(frame)) & 0xFF)
    return frame


def _compx_report_without_checksum(level=85, state=0, voltage_mv=4046):
    """A 16-byte revision with the report ID but without the checksum byte."""
    report = _compx_report(level=level, state=state, voltage_mv=voltage_mv)
    del report[16]
    return report


def test_aurora_new_with_report_id():
    report = [0] * 65
    report[1] = 0xA1
    report[4] = 0x02
    report[6] = 0x83
    report[7] = 1
    report[8] = 87

    assert lamzu._parse_aurora(report) == lamzu.BatteryReading(percent=87, state=1)
    assert lamzu._parse_aurora(report[1:]) == lamzu.BatteryReading(percent=87, state=1)


def test_aurora_legacy():
    report = [0] * 65
    report[1] = 0xA1
    report[2] = 0x02
    report[3] = 0x8F
    report[5] = 0
    report[6] = 42

    assert lamzu._parse_aurora(report) == lamzu.BatteryReading(percent=42, state=0)
    assert lamzu._parse_aurora(report[1:]) == lamzu.BatteryReading(percent=42, state=0)


def test_compx_with_checksum():
    report = [0] * 17
    report[0] = 0x08
    report[1] = 0x04
    report[6] = 66
    report[7] = 1
    report[16] = (0x55 - sum(report[:16])) & 0xFF

    assert lamzu._parse_compx(report) == lamzu.BatteryReading(
        percent=66, state=1, raw_percent=66
    )


def test_compx_rejects_bad_checksum():
    report = [0] * 17
    report[0] = 0x08
    report[1] = 0x04
    report[6] = 66
    report[7] = 1
    report[16] = 0x00

    assert lamzu._parse_compx(report) is None


def test_compx_without_report_id():
    reading = lamzu._parse_compx(
        _compx_report_without_id(level=55, voltage_mv=0)
    )

    assert reading == lamzu.BatteryReading(percent=55, state=0, raw_percent=55)


def test_compx_without_checksum_revision():
    reading = lamzu._parse_compx(
        _compx_report_without_checksum(level=85, voltage_mv=4046)
    )

    assert reading is not None
    assert reading.percent == 86
    assert reading.raw_percent == 85


def test_compx_rejects_arbitrary_report():
    # Any other input report of the control interface, even 64 bytes long,
    # must not be read as a battery percentage.
    report = [0] * 64
    report[0] = 0x04
    report[5] = 77
    report[6] = 0
    report[7] = 0x0F
    report[8] = 0xCE

    assert lamzu._parse_compx(report) is None


def test_compx_rejects_short_frame():
    assert lamzu._parse_compx([0x04, 0, 0, 0, 0, 55, 0, 0]) is None


def test_compx_rejects_broken_padding():
    report = _compx_report()
    report[3] = 0x7F
    report[16] = (0x55 - sum(report[:16])) & 0xFF

    assert lamzu._parse_compx(report) is None


def test_compx_tolerates_undefined_tail_bytes():
    # Bytes 10-15 are not documented: other models or a charging state may
    # put data there, so the parser must not depend on them.
    report = _compx_report(level=85, state=1, voltage_mv=4046)
    report[10:16] = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66]
    report[16] = (0x55 - sum(report[:16])) & 0xFF

    reading = lamzu._parse_compx(report)

    assert reading is not None
    assert reading.percent == 85
    assert reading.charging


def test_compx_rejects_wrong_checksum_without_report_id():
    frame = _compx_report_without_id()
    frame[15] ^= 0xFF

    assert lamzu._parse_compx(frame) is None


@pytest.mark.parametrize(
    ("voltage_mv", "expected"),
    [
        (4040, 85),
        (4046, 86),
        (4051, 87),
        (4060, 90),
        (4110, 99),
    ],
)
def test_compx_precise_percent_from_voltage(voltage_mv, expected):
    reading = lamzu._parse_compx(_compx_report(voltage_mv=voltage_mv))

    assert reading is not None
    assert reading.percent == expected
    assert reading.raw_percent == 85
    assert reading.voltage_mv == voltage_mv


def test_compx_voltage_hundred_is_capped_until_raw_hundred():
    below_full = lamzu._parse_compx(
        _compx_report(level=95, state=0, voltage_mv=4113)
    )
    full = lamzu._parse_compx(
        _compx_report(level=100, state=0, voltage_mv=4113)
    )

    assert below_full is not None
    assert below_full.percent == 99
    assert below_full.raw_percent == 95
    assert full is not None
    assert full.percent == 100
    assert full.raw_percent == 100


def test_compx_charging_uses_raw_above_eighty_percent():
    reading = lamzu._parse_compx(
        _compx_report(level=95, state=1, voltage_mv=4230)
    )

    assert reading is not None
    assert reading.percent == 95
    assert reading.raw_percent == 95
    assert reading.voltage_mv == 4230
    assert reading.charging


def test_compx_charging_keeps_voltage_precision_up_to_eighty_percent():
    reading = lamzu._parse_compx(
        _compx_report(level=80, state=1, voltage_mv=3990)
    )

    assert reading is not None
    assert reading.percent == 72
    assert reading.raw_percent == 80


def test_compx_charging_caps_above_eighty_percent():
    reading = lamzu._parse_compx(
        _compx_report(level=80, state=1, voltage_mv=4024)
    )

    assert reading is not None
    assert reading.percent == 80
    assert reading.raw_percent == 80


def test_compx_full_state_reports_hundred_only_with_raw_hundred():
    raw_95 = lamzu._parse_compx(
        _compx_report(level=95, state=2, voltage_mv=4200)
    )
    raw_100 = lamzu._parse_compx(
        _compx_report(level=100, state=2, voltage_mv=4200)
    )

    assert raw_95 is not None
    assert raw_95.percent == 99
    assert raw_95.full
    assert raw_100 is not None
    assert raw_100.percent == 100
    assert raw_100.full


def test_compx_discharging_keeps_voltage_precision():
    reading = lamzu._parse_compx(
        _compx_report(level=65, state=0, voltage_mv=3967)
    )

    assert reading is not None
    assert reading.percent == 66
    assert reading.raw_percent == 65


def test_compx_voltage_without_report_id():
    reading = lamzu._parse_compx(_compx_report_without_id(level=55, voltage_mv=4046))

    assert reading is not None
    assert reading.percent == 86
    assert reading.raw_percent == 55
    assert reading.voltage_mv == 4046


def test_compx_falls_back_to_raw_level_for_bad_voltage():
    reading = lamzu._parse_compx(_compx_report(voltage_mv=0))

    assert reading is not None
    assert reading.percent == 85
    assert reading.raw_percent == 85
    assert reading.voltage_mv is None


def test_voltage_curve_edges():
    assert lamzu.voltage_to_percent(None) is None
    assert lamzu.voltage_to_percent(0) is None
    assert lamzu.voltage_to_percent(2499) is None
    assert lamzu.voltage_to_percent(3050) == 0
    assert lamzu.voltage_to_percent(3420) == 5
    assert lamzu.voltage_to_percent(5001) is None


def test_full_reading_while_plugged():
    reading = lamzu.BatteryReading(percent=100, state=1)

    assert reading.full
    assert not reading.charging


def test_aurora_request_layouts():
    current = lamzu._aurora_request(True)
    assert len(current) == 65
    assert current[0] == 0
    assert current[3] == 0x02
    assert current[4] == 0x02
    assert current[6] == 0x83

    legacy = lamzu._aurora_request(False)
    assert len(legacy) == 65
    assert legacy[0] == 0
    assert legacy[2] == 0x02
    assert legacy[3] == 0x8F
    assert legacy[4] == 0x01


def test_model_name_detection():
    assert lamzu._model_from_product("LAMZU THORN") == "Thorn"
    assert lamzu._model_from_product("LAMZU MAYA X") == "Maya X"
    assert lamzu._model_from_product("LAMZU Maya X Wireless Mouse") == "Maya X"
    assert lamzu._model_from_product("LAMZU Maya X Wireless") == "Maya X"
    assert lamzu._model_from_product("LAMZU Thorn Gaming Mouse") == "Thorn"
    assert lamzu._model_from_product("Maya X") == "Maya X"
    assert lamzu._model_from_product("LAMZU Pro 1K Receiver") is None
    assert lamzu._model_from_product("LAMZU Wireless Receiver") is None
    assert lamzu._model_from_product("LAMZU 8K Receiver") is None
    assert lamzu._model_for(0x373E, 0x001E, "LAMZU 8K Receiver") == "Maya X"
    assert lamzu._model_for(0x37B0, 0x0010, "LAMZU 8K Receiver") == "Inca"


def _device(vendor_id, product_id, product="", protocol="compx", interface=1):
    return lamzu.DeviceInfo(
        path=b"path",
        vendor_id=vendor_id,
        product_id=product_id,
        product=product,
        manufacturer="LAMZU",
        usage_page=lamzu.USAGE_PAGE_COMPX,
        interface=interface,
        protocol=protocol,
        model=None,
    )


def test_wired_interface_is_ranked_before_a_dongle():
    cable = _device(0x3554, 0xF50F, "LAMZU THORN")
    dongle = _device(0x3554, 0xF50D, "LAMZU Pro 1K Receiver")

    assert lamzu._rank(cable) < lamzu._rank(dongle)


def test_wireless_name_does_not_demote_a_wired_interface():
    # A cable can report "LAMZU Maya X Wireless Mouse" as its product string.
    cable = _device(0x373E, 0x001C, "LAMZU Maya X Wireless Mouse")
    dongle = _device(0x373E, 0x001E, "LAMZU")

    assert lamzu._rank(cable) < lamzu._rank(dongle)


def test_receiver_name_is_ranked_after_a_plain_interface():
    plain = _device(0x3554, 0xF50D, "LAMZU")
    receiver = _device(0x3554, 0xF50E, "LAMZU Pro 1K Receiver")

    assert lamzu._rank(plain) < lamzu._rank(receiver)
