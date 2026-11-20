//! HID access through `hidapi`.
//!
//! The OS interface is hidden behind [`HidPort`] so the protocol layer can be
//! tested with synthetic reports and no hardware.

use std::ffi::CStr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use hidapi::HidApi;

use crate::protocol::{Protocol, RawDevice, aurora, compx};

/// Errors reported by the HID layer.
#[derive(Debug, thiserror::Error)]
pub enum HidError {
    /// The device list could not be read.
    #[error("HID enumeration failed: {0}")]
    Enumerate(String),
}

/// Read-only access to LAMZU control interfaces.
pub trait HidPort: Send + Sync {
    /// Every HID interface currently present.
    fn devices(&self) -> Result<Vec<RawDevice>, HidError>;
    /// Send the battery request and return the first parseable report.
    ///
    /// Stops as soon as a valid frame arrives so a poll costs milliseconds
    /// instead of the full timeout.
    fn exchange(&self, path: &CStr, protocol: Protocol, timeout: Duration) -> Vec<Vec<u8>>;
    /// Send the battery request and collect every report within the timeout.
    ///
    /// Diagnostics use this to show what the device really answered.
    fn probe(&self, path: &CStr, protocol: Protocol, timeout: Duration) -> Vec<Vec<u8>> {
        self.exchange(path, protocol, timeout)
    }
}

/// `hidapi`-backed implementation.
pub struct NativeHid {
    api: Mutex<HidApi>,
}

impl std::fmt::Debug for NativeHid {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("NativeHid")
    }
}

impl NativeHid {
    /// Open the HID subsystem.
    pub fn new() -> Result<Self, HidError> {
        let mut api = HidApi::new().map_err(|error| HidError::Enumerate(error.to_string()))?;
        let _ = api.refresh_devices();
        Ok(Self {
            api: Mutex::new(api),
        })
    }
}

impl HidPort for NativeHid {
    fn devices(&self) -> Result<Vec<RawDevice>, HidError> {
        let mut api = self
            .api
            .lock()
            .map_err(|_| HidError::Enumerate("HID lock poisoned".to_owned()))?;
        api.refresh_devices()
            .map_err(|error| HidError::Enumerate(error.to_string()))?;
        let mut devices = Vec::new();
        for info in api.device_list() {
            devices.push(RawDevice {
                path: info.path().to_bytes().to_vec(),
                vendor_id: info.vendor_id(),
                product_id: info.product_id(),
                product: info.product_string().unwrap_or_default().to_owned(),
                manufacturer: info.manufacturer_string().unwrap_or_default().to_owned(),
                usage_page: info.usage_page(),
                interface: info.interface_number(),
            });
        }
        Ok(devices)
    }

    fn exchange(&self, path: &CStr, protocol: Protocol, timeout: Duration) -> Vec<Vec<u8>> {
        self.read_reports(path, protocol, timeout, true)
    }

    fn probe(&self, path: &CStr, protocol: Protocol, timeout: Duration) -> Vec<Vec<u8>> {
        self.read_reports(path, protocol, timeout, false)
    }
}

impl NativeHid {
    fn read_reports(
        &self,
        path: &CStr,
        protocol: Protocol,
        timeout: Duration,
        stop_on_valid: bool,
    ) -> Vec<Vec<u8>> {
        let Ok(api) = self.api.lock() else {
            return Vec::new();
        };
        let device = match api.open_path(path) {
            Ok(device) => device,
            Err(error) => {
                log::debug!("could not open {}: {error}", path.to_string_lossy());
                return Vec::new();
            }
        };
        match protocol {
            Protocol::Compx => exchange_compx(&device, timeout, stop_on_valid),
            Protocol::Aurora => exchange_aurora(&device, timeout, stop_on_valid),
        }
    }
}

fn parses(protocol: Protocol, report: &[u8]) -> bool {
    match protocol {
        Protocol::Compx => compx::parse(report).is_some(),
        Protocol::Aurora => aurora::parse(report).is_some(),
    }
}

fn exchange_compx(
    device: &hidapi::HidDevice,
    timeout: Duration,
    stop_on_valid: bool,
) -> Vec<Vec<u8>> {
    let mut reports = Vec::new();
    if device.write(&compx::REQUEST).is_err() {
        log::debug!("Compx request write failed");
        return reports;
    }
    let mut buffer = [0_u8; 65];
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        match device.read_timeout(&mut buffer, 40) {
            Ok(count) if count > 0 => {
                reports.push(buffer[..count].to_vec());
                if stop_on_valid && parses(Protocol::Compx, &buffer[..count]) {
                    return reports;
                }
            }
            Ok(_) => {}
            Err(error) => {
                log::debug!("Compx read failed: {error}");
                break;
            }
        }
    }
    // Some revisions answer with a feature report instead of an input report.
    if let Ok(count) = device.get_feature_report(&mut buffer)
        && count > 0
    {
        reports.push(buffer[..count].to_vec());
    }
    reports
}

fn exchange_aurora(
    device: &hidapi::HidDevice,
    timeout: Duration,
    stop_on_valid: bool,
) -> Vec<Vec<u8>> {
    let mut reports = Vec::new();
    let per_revision = (timeout / 2).max(Duration::from_millis(150));
    let mut buffer = [0_u8; aurora::REPORT_LEN];

    for new_protocol in [true, false] {
        let request = aurora::request(new_protocol);
        let sent = device.send_feature_report(&request).is_ok() || device.write(&request).is_ok();
        if !sent {
            log::debug!("Aurora request could not be sent (new_protocol={new_protocol})");
            continue;
        }
        std::thread::sleep(Duration::from_millis(20));
        let deadline = Instant::now() + per_revision;
        while Instant::now() < deadline {
            if let Ok(count) = device.get_feature_report(&mut buffer)
                && count > 0
            {
                reports.push(buffer[..count].to_vec());
                if stop_on_valid && parses(Protocol::Aurora, &buffer[..count]) {
                    return reports;
                }
            }
            if let Ok(count) = device.read_timeout(&mut buffer, 30)
                && count > 0
            {
                reports.push(buffer[..count].to_vec());
                if stop_on_valid && parses(Protocol::Aurora, &buffer[..count]) {
                    return reports;
                }
            }
        }
    }
    reports
}

/// A HID port that never reaches the hardware, used before `hidapi` is opened
/// and by diagnostics when enumeration fails.
#[derive(Debug, Default)]
pub struct NoHid;

impl HidPort for NoHid {
    fn devices(&self) -> Result<Vec<RawDevice>, HidError> {
        Ok(Vec::new())
    }

    fn exchange(&self, _path: &CStr, _protocol: Protocol, _timeout: Duration) -> Vec<Vec<u8>> {
        Vec::new()
    }
}
