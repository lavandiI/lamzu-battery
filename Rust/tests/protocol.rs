//! Protocol parsing, battery curve and interface ranking.

use lamzu_battery::protocol::{
    self, BatteryReading, ChargeState, DeviceInfo, Percent, Protocol, RawDevice, aurora, compx,
};

fn compx_frame(level: u8, state: u8, voltage_mv: u16) -> Vec<u8> {
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

fn compx_without_id(level: u8, state: u8, voltage_mv: u16) -> Vec<u8> {
    let mut frame = vec![0x04, 0x00, 0x00, 0x00, 0x02, level, state];
    frame.push((voltage_mv >> 8) as u8);
    frame.push(voltage_mv as u8);
    frame.extend_from_slice(&[0_u8; 6]);
    let sum: u8 = frame
        .iter()
        .fold(0x08_u8, |acc, byte| acc.wrapping_add(*byte));
    frame.push(0x55_u8.wrapping_sub(sum));
    assert_eq!(frame.len(), 16);
    frame
}

fn read(frame: &[u8]) -> Option<BatteryReading> {
    compx::parse(frame)
}

#[test]
fn compx_frame_with_checksum_parses() {
    let reading = read(&compx_frame(66, 1, 0)).expect("valid frame");
    assert_eq!(reading.percent.get(), 66);
    assert_eq!(reading.state, ChargeState::Charging);
    assert_eq!(reading.raw_percent.map(Percent::get), Some(66));
}

#[test]
fn compx_bad_checksum_is_rejected() {
    let mut frame = compx_frame(66, 1, 0);
    frame[16] = 0;
    assert!(read(&frame).is_none());
}

#[test]
fn compx_frame_without_report_id_parses() {
    let reading = read(&compx_without_id(55, 0, 0)).expect("valid frame");
    assert_eq!(reading.percent.get(), 55);
    assert_eq!(reading.raw_percent.map(Percent::get), Some(55));
}

#[test]
fn compx_revision_without_checksum_parses() {
    let mut frame = compx_frame(85, 0, 4046);
    frame.truncate(16);
    let reading = read(&frame).expect("valid frame");
    assert_eq!(reading.percent.get(), 86);
    assert_eq!(reading.raw_percent.map(Percent::get), Some(85));
}

#[test]
fn compx_rejects_arbitrary_report() {
    let mut report = vec![0_u8; 64];
    report[0] = 0x04;
    report[5] = 77;
    report[7] = 0x0F;
    report[8] = 0xCE;
    assert!(read(&report).is_none());
}

#[test]
fn compx_rejects_short_frame() {
    assert!(read(&[0x04, 0, 0, 0, 0, 55, 0, 0]).is_none());
}

#[test]
fn compx_rejects_broken_padding() {
    let mut frame = compx_frame(85, 0, 4046);
    frame[3] = 0x7F;
    let sum: u8 = frame[..16]
        .iter()
        .fold(0_u8, |acc, byte| acc.wrapping_add(*byte));
    frame[16] = 0x55_u8.wrapping_sub(sum);
    assert!(read(&frame).is_none());
}

#[test]
fn compx_tolerates_undefined_tail() {
    let mut frame = compx_frame(85, 1, 4046);
    frame[10..16].copy_from_slice(&[0x11, 0x22, 0x33, 0x44, 0x55, 0x66]);
    let sum: u8 = frame[..16]
        .iter()
        .fold(0_u8, |acc, byte| acc.wrapping_add(*byte));
    frame[16] = 0x55_u8.wrapping_sub(sum);
    let reading = read(&frame).expect("valid frame");
    assert_eq!(reading.percent.get(), 85);
    assert!(reading.charging());
}

#[test]
fn compx_rejects_wrong_checksum_without_report_id() {
    let mut frame = compx_without_id(55, 0, 4046);
    frame[15] ^= 0xFF;
    assert!(read(&frame).is_none());
}

#[test]
fn compx_voltage_gives_precise_percent() {
    for (voltage, expected) in [(4040, 85), (4046, 86), (4051, 87), (4060, 90), (4110, 99)] {
        let reading = read(&compx_frame(85, 0, voltage)).expect("valid frame");
        assert_eq!(reading.percent.get(), expected, "voltage {voltage}");
        assert_eq!(reading.voltage_mv, Some(voltage));
    }
}

#[test]
fn compx_hundred_needs_raw_hundred() {
    let below = read(&compx_frame(95, 0, 4113)).expect("valid frame");
    let full = read(&compx_frame(100, 0, 4113)).expect("valid frame");
    assert_eq!(below.percent.get(), 99);
    assert_eq!(full.percent.get(), 100);
}

#[test]
fn compx_charging_above_eighty_uses_device_level() {
    let reading = read(&compx_frame(95, 1, 4230)).expect("valid frame");
    assert_eq!(reading.percent.get(), 95);
    assert!(reading.charging());
}

#[test]
fn compx_charging_keeps_precision_below_eighty() {
    let reading = read(&compx_frame(80, 1, 3990)).expect("valid frame");
    assert_eq!(reading.percent.get(), 72);
}

#[test]
fn compx_charging_caps_at_device_level() {
    let reading = read(&compx_frame(80, 1, 4024)).expect("valid frame");
    assert_eq!(reading.percent.get(), 80);
}

#[test]
fn compx_full_state_reports_hundred_only_with_raw() {
    let raw_95 = read(&compx_frame(95, 2, 4200)).expect("valid frame");
    let raw_100 = read(&compx_frame(100, 2, 4200)).expect("valid frame");
    assert_eq!(raw_95.percent.get(), 99);
    assert!(raw_95.full());
    assert_eq!(raw_100.percent.get(), 100);
}

#[test]
fn compx_discharging_keeps_precision() {
    let reading = read(&compx_frame(65, 0, 3967)).expect("valid frame");
    assert_eq!(reading.percent.get(), 66);
}

#[test]
fn compx_falls_back_to_raw_level() {
    let reading = read(&compx_frame(85, 0, 0)).expect("valid frame");
    assert_eq!(reading.percent.get(), 85);
    assert_eq!(reading.voltage_mv, None);
}

#[test]
fn voltage_curve_edges() {
    assert_eq!(compx::voltage_to_percent(None), None);
    assert_eq!(compx::voltage_to_percent(Some(0)), None);
    assert_eq!(compx::voltage_to_percent(Some(2499)), None);
    assert_eq!(
        compx::voltage_to_percent(Some(3050)).map(Percent::get),
        Some(0)
    );
    assert_eq!(
        compx::voltage_to_percent(Some(3420)).map(Percent::get),
        Some(5)
    );
    assert_eq!(compx::voltage_to_percent(Some(5001)), None);
    for voltage in 3050..=4110 {
        let percent = compx::voltage_to_percent(Some(voltage)).expect("inside the curve");
        assert!(percent.get() <= 100);
    }
}

#[test]
fn aurora_current_layout_with_and_without_id() {
    let mut report = vec![0_u8; 65];
    report[1] = 0xA1;
    report[4] = 0x02;
    report[6] = 0x83;
    report[7] = 1;
    report[8] = 87;
    let with_id = aurora::parse(&report).expect("valid frame");
    assert_eq!(with_id.percent.get(), 87);
    assert_eq!(with_id.state, ChargeState::Charging);
    let without_id = aurora::parse(&report[1..]).expect("valid frame");
    assert_eq!(without_id.percent.get(), 87);
}

#[test]
fn aurora_legacy_layout_with_and_without_id() {
    let mut report = vec![0_u8; 65];
    report[1] = 0xA1;
    report[2] = 0x02;
    report[3] = 0x8F;
    report[6] = 42;
    assert_eq!(
        aurora::parse(&report).map(|reading| reading.percent.get()),
        Some(42)
    );
    assert_eq!(
        aurora::parse(&report[1..]).map(|reading| reading.percent.get()),
        Some(42)
    );
}

#[test]
fn aurora_requests_have_the_expected_layout() {
    let current = aurora::request(true);
    assert_eq!(current.len(), 65);
    assert_eq!(current[0], 0);
    assert_eq!((current[3], current[4], current[6]), (0x02, 0x02, 0x83));
    let legacy = aurora::request(false);
    assert_eq!(legacy.len(), 65);
    assert_eq!(legacy[0], 0);
    assert_eq!((legacy[2], legacy[3], legacy[4]), (0x02, 0x8F, 0x01));
}

#[test]
fn model_names_are_extracted_from_product_strings() {
    for (product, expected) in [
        ("LAMZU THORN", Some("Thorn")),
        ("LAMZU MAYA X", Some("Maya X")),
        ("LAMZU Maya X Wireless Mouse", Some("Maya X")),
        ("LAMZU Maya X Wireless", Some("Maya X")),
        ("LAMZU Thorn Gaming Mouse", Some("Thorn")),
        ("Maya X", Some("Maya X")),
        ("LAMZU Pro 1K Receiver", None),
        ("LAMZU Wireless Receiver", None),
        ("LAMZU 8K Receiver", None),
    ] {
        assert_eq!(
            protocol::model_from_product(product).as_deref(),
            expected,
            "product {product}"
        );
    }
    assert_eq!(
        protocol::model_for(0x373E, 0x001E, "LAMZU 8K Receiver").as_deref(),
        Some("Maya X")
    );
    assert_eq!(
        protocol::model_for(0x37B0, 0x0010, "LAMZU 8K Receiver").as_deref(),
        Some("Inca")
    );
}

fn raw(
    vendor: u16,
    product: u16,
    product_string: &str,
    usage_page: u16,
    interface: i32,
) -> RawDevice {
    RawDevice {
        path: format!("{vendor:04x}:{product:04x}:{product_string}:{interface}").into_bytes(),
        vendor_id: vendor,
        product_id: product,
        product: product_string.to_owned(),
        manufacturer: "LAMZU".to_owned(),
        usage_page,
        interface,
    }
}

fn ids(candidates: &[DeviceInfo]) -> Vec<(u16, u16, Protocol)> {
    candidates
        .iter()
        .map(|candidate| {
            (
                candidate.vendor_id,
                candidate.product_id,
                candidate.protocol,
            )
        })
        .collect()
}

#[test]
fn wired_interface_is_ranked_before_a_dongle() {
    let devices = vec![
        raw(0x3554, 0xF50D, "LAMZU Pro 1K Receiver", 0xFF02, 1),
        raw(0x3554, 0xF50F, "LAMZU THORN", 0xFF02, 1),
    ];
    let candidates = protocol::select_candidates(&devices);
    assert_eq!(
        ids(&candidates),
        vec![
            (0x3554, 0xF50F, Protocol::Compx),
            (0x3554, 0xF50D, Protocol::Compx)
        ]
    );
}

#[test]
fn plain_interface_is_ranked_before_a_receiver() {
    let devices = vec![
        raw(0x3554, 0xF50E, "LAMZU Pro 1K Receiver", 0xFF02, 1),
        raw(0x3554, 0xF50D, "LAMZU", 0xFF02, 1),
    ];
    let candidates = protocol::select_candidates(&devices);
    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0].product_id, 0xF50D);
}

#[test]
fn aurora_control_interface_wins() {
    let devices = vec![
        raw(0x37B0, 0x0010, "LAMZU INCA", 0xFFFF, 0),
        raw(0x37B0, 0x0010, "LAMZU INCA", 0xFFFF, 2),
        raw(0x37B0, 0x0010, "LAMZU INCA", 0x0001, 1),
    ];
    let candidates = protocol::select_candidates(&devices);
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].interface, 2);
    assert_eq!(candidates[0].protocol, Protocol::Aurora);
}

#[test]
fn unknown_vendors_and_pages_are_ignored() {
    let devices = vec![
        raw(0x1234, 0x0001, "Keyboard", 0x0001, 0),
        raw(0x3554, 0xF50D, "LAMZU", 0x0001, 1),
    ];
    assert!(protocol::select_candidates(&devices).is_empty());
    assert_eq!(protocol::model_from_product("LAMZU Wireless"), None);
}

#[test]
fn battery_reading_flags_match_the_python_semantics() {
    let full = BatteryReading {
        percent: Percent::MAX,
        state: ChargeState::Charging,
        model: None,
        protocol: Protocol::Compx,
        product: None,
        voltage_mv: None,
        raw_percent: None,
    };
    assert!(full.full());
    assert!(!full.charging());

    let charging = BatteryReading {
        percent: Percent::new(50).expect("valid"),
        state: ChargeState::Charging,
        ..full.clone()
    };
    assert!(charging.charging());
    assert!(!charging.full());
}

mod properties {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn compx_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..90)) {
            if let Some(reading) = compx::parse(&bytes) {
                prop_assert!(reading.percent.get() <= 100);
            }
        }

        #[test]
        fn aurora_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..90)) {
            if let Some(reading) = aurora::parse(&bytes) {
                prop_assert!(reading.percent.get() <= 100);
            }
        }
    }
}
