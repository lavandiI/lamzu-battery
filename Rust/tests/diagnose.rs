//! Diagnostics output and exit codes with a synthetic HID device.

use std::ffi::CStr;
use std::path::Path;
use std::time::Duration;

use lamzu_battery::diagnostics;
use lamzu_battery::hid::{HidError, HidPort};
use lamzu_battery::i18n::Language;
use lamzu_battery::protocol::{Protocol, RawDevice};

struct MockHid {
    devices: Vec<RawDevice>,
    reports: Vec<Vec<u8>>,
}

impl HidPort for MockHid {
    fn devices(&self) -> Result<Vec<RawDevice>, HidError> {
        Ok(self.devices.clone())
    }

    fn exchange(&self, _path: &CStr, _protocol: Protocol, _timeout: Duration) -> Vec<Vec<u8>> {
        self.reports.clone()
    }
}

fn frame(level: u8, state: u8, voltage_mv: u16) -> Vec<u8> {
    let mut report = vec![0_u8; 17];
    report[0] = 0x08;
    report[1] = 0x04;
    report[5] = 0x02;
    report[6] = level;
    report[7] = state;
    report[8] = (voltage_mv >> 8) as u8;
    report[9] = voltage_mv as u8;
    let sum: u8 = report[..16]
        .iter()
        .fold(0_u8, |acc, byte| acc.wrapping_add(*byte));
    report[16] = 0x55_u8.wrapping_sub(sum);
    report
}

fn thorn() -> RawDevice {
    RawDevice {
        path: b"device-1".to_vec(),
        vendor_id: 0x3554,
        product_id: 0xF50F,
        product: "LAMZU THORN".to_owned(),
        manufacturer: "LAMZU".to_owned(),
        usage_page: 0xFF02,
        interface: 1,
    }
}

#[test]
fn missing_hardware_exits_with_one() {
    let hid = MockHid {
        devices: Vec::new(),
        reports: Vec::new(),
    };
    let code = diagnostics::run(Language::En, false, &hid, Path::new("LamzuBattery.exe"));
    assert_eq!(code, 1);
}

#[test]
fn answering_mouse_exits_with_zero() {
    let hid = MockHid {
        devices: vec![thorn()],
        reports: vec![frame(95, 0, 4100)],
    };
    let code = diagnostics::run(Language::En, false, &hid, Path::new("LamzuBattery.exe"));
    assert_eq!(code, 0);
}

#[test]
fn raw_mode_is_supported() {
    let hid = MockHid {
        devices: vec![thorn()],
        reports: vec![frame(80, 1, 4000)],
    };
    let code = diagnostics::run(Language::Ru, true, &hid, Path::new("LamzuBattery.exe"));
    assert_eq!(code, 0);
}

#[test]
fn a_silent_mouse_exits_with_one() {
    let hid = MockHid {
        devices: vec![thorn()],
        reports: Vec::new(),
    };
    let code = diagnostics::run(Language::En, false, &hid, Path::new("LamzuBattery.exe"));
    assert_eq!(code, 1);
}
