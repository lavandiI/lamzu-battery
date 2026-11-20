//! Aurora protocol: 65-byte feature reports with a reply tag.

use super::{BatteryReading, ChargeState, Percent, Protocol};

/// Report id used by Aurora feature reports.
pub const REPORT_ID: u8 = 0x00;
/// Reply tag of a valid Aurora answer.
pub const REPLY_TAG: u8 = 0xA1;
/// Current battery command.
pub const BATTERY_COMMAND: u8 = 0x83;
/// Legacy battery command.
pub const LEGACY_COMMAND: u8 = 0x8F;
/// Length of an Aurora report.
pub const REPORT_LEN: usize = 65;

/// Build the read-only battery request for the current or legacy revision.
#[must_use]
pub fn request(new_protocol: bool) -> [u8; REPORT_LEN] {
    let mut report = [0_u8; REPORT_LEN];
    report[0] = REPORT_ID;
    if new_protocol {
        report[3] = 0x02; // mouse
        report[4] = 0x02; // payload length
        report[6] = BATTERY_COMMAND;
    } else {
        report[2] = 0x02; // mouse
        report[3] = LEGACY_COMMAND;
        report[4] = 0x01; // wireless receiver target
    }
    report
}

fn valid(percent: u8, state: u8) -> Option<BatteryReading> {
    Some(BatteryReading {
        percent: Percent::new(percent)?,
        state: ChargeState::from_raw(state),
        model: None,
        protocol: Protocol::Aurora,
        product: None,
        voltage_mv: None,
        raw_percent: None,
    })
}

/// Parse an Aurora answer with or without a report id, current or legacy.
#[must_use]
pub fn parse(data: &[u8]) -> Option<BatteryReading> {
    let (percent, state) =
        if data.len() >= 9 && data[1] == REPLY_TAG && data[4] == 0x02 && data[6] == BATTERY_COMMAND
        {
            (data[8], data[7])
        } else if data.len() >= 8
            && data[0] == REPLY_TAG
            && data[3] == 0x02
            && data[5] == BATTERY_COMMAND
        {
            (data[7], data[6])
        } else if data.len() >= 7
            && data[1] == REPLY_TAG
            && data[2] == 0x02
            && data[3] == LEGACY_COMMAND
        {
            (data[6], data[5])
        } else if data.len() >= 6
            && data[0] == REPLY_TAG
            && data[1] == 0x02
            && data[2] == LEGACY_COMMAND
        {
            (data[5], data[4])
        } else {
            return None;
        };

    let raw_state_ok = state <= 3;
    let reading = valid(percent, state);
    if reading.is_none() || !raw_state_ok {
        log::debug!("unexpected Aurora payload: {data:?}");
        return None;
    }
    reading
}
