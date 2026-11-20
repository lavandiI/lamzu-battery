//! Diagnostics: list LAMZU HID interfaces and read the battery.
//!
//! This is the Rust equivalent of the Python `diagnose.py` script and is meant
//! to be attached to bug reports. It never writes to the hardware state.

use std::path::Path;

use crate::hid::HidPort;
use crate::i18n::{self, Key, Language};
use crate::protocol::{self, BatteryReading};

fn hex_report(report: &[u8]) -> String {
    report
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn print_reading(language: Language, reading: &BatteryReading) {
    let model = reading
        .model
        .as_deref()
        .map_or_else(|| "LAMZU".to_owned(), |model| format!("LAMZU {model}"));
    println!(
        "{}",
        i18n::tr_args(
            language,
            Key::DiagnoseBattery,
            &[
                ("percent", &reading.percent.to_string()),
                ("state", &i18n::state_text(language, reading.state)),
                ("model", &model),
                ("protocol", reading.protocol.display_name()),
            ],
        )
    );
    let Some(raw_percent) = reading.raw_percent else {
        return;
    };
    let mut line = i18n::tr_args(
        language,
        Key::DiagnoseRawLevel,
        &[("percent", &raw_percent.to_string())],
    );
    if let Some(voltage) = reading.voltage_mv {
        let calculated = protocol::compx::voltage_to_percent(Some(voltage))
            .map_or_else(|| "—".to_owned(), |percent| percent.to_string());
        line.push_str("; ");
        line.push_str(&i18n::tr_args(
            language,
            Key::DiagnoseVoltage,
            &[("voltage", &voltage.to_string()), ("percent", &calculated)],
        ));
    }
    line.push_str("; ");
    line.push_str(&i18n::tr_args(
        language,
        Key::DiagnoseShown,
        &[("percent", &reading.percent.to_string())],
    ));
    println!("  {line}");
}

/// Print the diagnostic report and return the process exit code.
#[must_use]
pub fn run(language: Language, raw: bool, port: &dyn HidPort, exe: &Path) -> u8 {
    println!(
        "{}",
        i18n::tr_args(
            language,
            Key::DiagnoseVersion,
            &[("version", crate::VERSION)]
        )
    );
    println!(
        "{}",
        i18n::tr_args(
            language,
            Key::DiagnoseExecutable,
            &[("path", &exe.to_string_lossy())],
        )
    );

    let devices = match port.devices() {
        Ok(devices) => devices,
        Err(error) => {
            println!(
                "{}",
                i18n::tr_args(
                    language,
                    Key::DiagnoseHidError,
                    &[("error", &error.to_string())]
                )
            );
            Vec::new()
        }
    };

    println!("{}", i18n::tr(language, Key::DiagnoseHeader));
    let controls = protocol::select_candidates(&devices);
    let mut found = 0_usize;
    for device in &devices {
        if !protocol::is_lamzu(device) {
            continue;
        }
        found += 1;
        let control = controls
            .iter()
            .find(|candidate| candidate.path.as_bytes() == device.path.as_slice());
        let marker = control.map_or_else(String::new, |candidate| {
            i18n::tr_args(
                language,
                Key::DiagnoseControl,
                &[
                    ("protocol", candidate.protocol.display_name()),
                    (
                        "model",
                        candidate
                            .model
                            .as_deref()
                            .unwrap_or(i18n::tr(language, Key::DiagnoseModelUnknown)),
                    ),
                ],
            )
        });
        println!(
            "  {:04X}:{:04X}  iface {}  usage_page 0x{:04X}  {:?}{marker}",
            device.vendor_id,
            device.product_id,
            device.interface,
            device.usage_page,
            device.product
        );
    }
    if found == 0 {
        println!("{}", i18n::tr(language, Key::DiagnoseNotFound));
    }

    if raw {
        println!();
        println!("{}", i18n::tr(language, Key::DiagnoseRawHeader));
        if controls.is_empty() {
            println!("{}", i18n::tr(language, Key::DiagnoseRawNone));
        }
        for control in &controls {
            println!(
                "  {}  iface {}  ({})",
                control.usb_id(),
                control.interface,
                control.protocol
            );
            let reports = protocol::probe_reports(port, control, crate::HID_TIMEOUT);
            if reports.is_empty() {
                println!("{}", i18n::tr(language, Key::DiagnoseRawNone));
                continue;
            }
            for report in reports {
                println!("    {}", hex_report(&report));
            }
        }
    }

    println!();
    match protocol::read_battery(port, crate::HID_TIMEOUT) {
        Some(reading) => print_reading(language, &reading),
        None => {
            println!("{}", i18n::tr(language, Key::DiagnoseNoResponse));
            return 1;
        }
    }
    if found > 0 { 0 } else { 1 }
}
