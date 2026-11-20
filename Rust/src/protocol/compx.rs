//! Compx protocol: 17-byte reports with a checksum, plus the official battery
//! voltage curve used to turn millivolts into a precise percentage.

use super::{BatteryReading, ChargeState, Percent};

/// Report id of the Compx control report.
pub const REPORT_ID: u8 = 0x08;
/// Command that requests the battery state.
pub const BATTERY_COMMAND: u8 = 0x04;
/// Read-only battery request.
pub const REQUEST: [u8; 17] = [
    0x08, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x49,
];

/// Official `BatteryParam` curve from LAMZU's `Config.ini`: 3050 mV = 0% and
/// 4110 mV = 100%, in equal 5% steps.
pub const VOLTAGE_CURVE_MV: [u16; 21] = [
    3050, 3420, 3480, 3540, 3600, 3660, 3720, 3760, 3800, 3840, 3880, 3920, 3940, 3960, 3980, 4000,
    4020, 4040, 4060, 4080, 4110,
];
/// Lowest plausible battery voltage.
pub const MIN_VALID_VOLTAGE_MV: u16 = 2500;
/// Highest plausible battery voltage.
pub const MAX_VALID_VOLTAGE_MV: u16 = 5000;

/// Interpolate the official curve down to 1% steps.
#[must_use]
pub fn voltage_to_percent(voltage_mv: Option<u16>) -> Option<Percent> {
    let voltage = voltage_mv?;
    if !(MIN_VALID_VOLTAGE_MV..=MAX_VALID_VOLTAGE_MV).contains(&voltage) {
        return None;
    }
    if voltage <= VOLTAGE_CURVE_MV[0] {
        return Percent::new(0);
    }
    if voltage >= VOLTAGE_CURVE_MV[VOLTAGE_CURVE_MV.len() - 1] {
        return Percent::new(100);
    }
    for (index, window) in VOLTAGE_CURVE_MV.windows(2).enumerate() {
        let (low, high) = (window[0], window[1]);
        if voltage <= high {
            let base = u8::try_from(index).ok()?.checked_mul(5)?;
            let step = (voltage - low) * 5 / (high - low);
            return Percent::new(base + u8::try_from(step).ok()?);
        }
    }
    Percent::new(100)
}

fn voltage_at(data: &[u8], offset: usize) -> Option<u16> {
    let high = *data.get(offset)?;
    let low = *data.get(offset + 1)?;
    let voltage = u16::from(high) << 8 | u16::from(low);
    (MIN_VALID_VOLTAGE_MV..=MAX_VALID_VOLTAGE_MV)
        .contains(&voltage)
        .then_some(voltage)
}

fn padding_ok(data: &[u8], start: usize, stop: usize) -> bool {
    data.get(start..stop)
        .is_some_and(|slice| slice.iter().all(|&byte| byte == 0))
}

/// Parse a Compx answer in any of its three known layouts: 17 bytes with a
/// report id and checksum, 16 bytes with a report id and no checksum, and
/// 16 bytes with the report id stripped by the HID stack.
#[must_use]
pub fn parse(data: &[u8]) -> Option<BatteryReading> {
    let (raw_level, state, voltage_mv) =
        if data.len() >= 17 && data[0] == REPORT_ID && data[1] == BATTERY_COMMAND {
            if !padding_ok(data, 2, 5) {
                log::debug!("unexpected Compx frame: {data:?}");
                return None;
            }
            let expected = 0x55_u8.wrapping_sub(
                data[..16]
                    .iter()
                    .fold(0_u8, |sum, &byte| sum.wrapping_add(byte)),
            );
            if data[16] != expected {
                log::debug!(
                    "bad Compx checksum: got 0x{:02x}, expected 0x{expected:02x}",
                    data[16]
                );
                return None;
            }
            (data[6], data[7], voltage_at(data, 8))
        } else if data.len() == 16 && data[0] == REPORT_ID && data[1] == BATTERY_COMMAND {
            if !padding_ok(data, 2, 5) {
                log::debug!("unexpected Compx frame: {data:?}");
                return None;
            }
            (data[6], data[7], voltage_at(data, 8))
        } else if data.len() == 16 && data[0] == BATTERY_COMMAND {
            if !padding_ok(data, 1, 4) {
                log::debug!("unexpected Compx frame: {data:?}");
                return None;
            }
            let expected = 0x55_u8.wrapping_sub(REPORT_ID).wrapping_sub(
                data[..15]
                    .iter()
                    .fold(0_u8, |sum, &byte| sum.wrapping_add(byte)),
            );
            if data[15] != expected {
                log::debug!(
                    "bad Compx checksum without report id: got 0x{:02x}, expected 0x{expected:02x}",
                    data[15]
                );
                return None;
            }
            (data[5], data[6], voltage_at(data, 7))
        } else {
            return None;
        };

    let raw_percent = Percent::new(raw_level)?;
    let state = ChargeState::from_raw(state);
    if matches!(state, ChargeState::Other(raw) if raw > 3) {
        log::debug!("unexpected Compx payload: {data:?}");
        return None;
    }

    let mut percent = voltage_to_percent(voltage_mv).unwrap_or(raw_percent);

    // Above 80% while charging the charger holds the voltage high and
    // overestimates the remaining capacity, so the device level is used.
    if state == ChargeState::Charging
        && percent > Percent::new(80)?
        && raw_percent >= Percent::new(80)?
    {
        percent = percent.min(raw_percent);
    }
    if raw_percent < Percent::MAX {
        percent = percent.min(Percent::new(99)?);
    } else if state == ChargeState::Full {
        percent = Percent::MAX;
    }

    Some(BatteryReading {
        percent,
        state,
        model: None,
        protocol: super::Protocol::Compx,
        product: None,
        voltage_mv,
        raw_percent: Some(raw_percent),
    })
}
