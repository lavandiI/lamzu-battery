"""Protocol parsing and request layout tests.

These tests never touch real hardware: they build synthetic HID reports and
check the parsers used by the tray application.
"""

import lamzu


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

    assert lamzu._parse_compx(report) == lamzu.BatteryReading(percent=66, state=1)


def test_compx_rejects_bad_checksum():
    report = [0] * 17
    report[0] = 0x08
    report[1] = 0x04
    report[6] = 66
    report[7] = 1
    report[16] = 0x00

    assert lamzu._parse_compx(report) is None


def test_compx_without_report_id():
    report = [0x04, 0, 0, 0, 0, 55, 0, 0]

    assert lamzu._parse_compx(report) == lamzu.BatteryReading(percent=55, state=0)


def test_full_reading_while_plugged():
    reading = lamzu.BatteryReading(percent=100, state=1)

    assert reading.full
    assert not reading.charging
    assert reading.state_text == "заряжена"


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
    assert lamzu._model_from_product("LAMZU Pro 1K Receiver") is None
    assert lamzu._model_from_product("LAMZU 8K Receiver") is None
    assert lamzu._model_for(0x373E, 0x001E, "LAMZU 8K Receiver") == "Maya X"
    assert lamzu._model_for(0x37B0, 0x0010, "LAMZU 8K Receiver") == "Inca"
