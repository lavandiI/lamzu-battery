//! Battery protocol support for LAMZU mice.
//!
//! Two protocol families exist: **Compx** (usage page `0xFF02`, report id 8) and
//! **Aurora** (usage page `0xFFFF`, interface 2). Devices are found by vendor id
//! or by a `LAMZU` string in their descriptor, so new models work without code
//! changes. Only read-only battery queries are sent.

pub mod aurora;
pub mod compx;

use std::collections::HashSet;
use std::ffi::{CStr, CString};
use std::fmt;
use std::time::Duration;

use crate::hid::HidPort;

/// Vendor ids known to belong to LAMZU.
pub const LAMZU_VENDOR_IDS: [u16; 3] = [0x3554, 0x373E, 0x37B0];
/// Usage page of the Compx control interface.
pub const USAGE_PAGE_COMPX: u16 = 0xFF02;
/// Usage page of the Aurora control interface.
pub const USAGE_PAGE_AURORA: u16 = 0xFFFF;
/// Control interface of Aurora mice.
pub const AURORA_INTERFACE: i32 = 2;

/// A validated battery percentage between 0 and 100.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Percent(u8);

impl Percent {
    /// Fully charged.
    pub const MAX: Self = Self(100);

    /// Validate a raw percentage.
    #[must_use]
    pub const fn new(value: u8) -> Option<Self> {
        if value <= 100 {
            Some(Self(value))
        } else {
            None
        }
    }

    /// Raw value.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }

    /// Saturate an arbitrary value into the valid range.
    #[must_use]
    pub const fn saturating(value: u8) -> Self {
        Self(if value > 100 { 100 } else { value })
    }
}

impl fmt::Display for Percent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// Charging state reported by the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChargeState {
    /// Running on battery.
    Discharging,
    /// Cable is connected and the battery is charging.
    Charging,
    /// The device reports a full battery.
    Full,
    /// An undocumented raw value.
    Other(u8),
}

impl ChargeState {
    /// Decode the raw state byte.
    #[must_use]
    pub const fn from_raw(raw: u8) -> Self {
        match raw {
            0 => Self::Discharging,
            1 => Self::Charging,
            2 => Self::Full,
            other => Self::Other(other),
        }
    }

    /// Raw state byte.
    #[must_use]
    pub const fn raw(self) -> u8 {
        match self {
            Self::Discharging => 0,
            Self::Charging => 1,
            Self::Full => 2,
            Self::Other(raw) => raw,
        }
    }
}

/// Protocol family of a control interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Protocol {
    /// Compx mice and dongles.
    Compx,
    /// Aurora mice and dongles.
    Aurora,
}

impl Protocol {
    /// Stable lowercase name used in logs and status files.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Compx => "compx",
            Self::Aurora => "aurora",
        }
    }

    /// Human readable name used in diagnostics.
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Compx => "Compx",
            Self::Aurora => "Aurora",
        }
    }
}

impl fmt::Display for Protocol {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One accepted battery reading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatteryReading {
    /// Percentage shown in the tray.
    pub percent: Percent,
    /// State reported by the device.
    pub state: ChargeState,
    /// Model name, when detected.
    pub model: Option<String>,
    /// Protocol that produced the reading.
    pub protocol: Protocol,
    /// Raw product string of the interface.
    pub product: Option<String>,
    /// Battery voltage in millivolts (Compx only).
    pub voltage_mv: Option<u16>,
    /// Coarse percentage reported by the device (Compx only).
    pub raw_percent: Option<Percent>,
}

impl BatteryReading {
    /// True while the battery is actually charging.
    #[must_use]
    pub fn charging(&self) -> bool {
        self.state == ChargeState::Charging && self.percent < Percent::MAX
    }

    /// True when the device reports a full battery.
    #[must_use]
    pub fn full(&self) -> bool {
        self.state == ChargeState::Full
            || (self.state == ChargeState::Charging && self.percent >= Percent::MAX)
    }
}

/// A HID interface as reported by the operating system.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawDevice {
    /// Opaque device path.
    pub path: Vec<u8>,
    /// USB vendor id.
    pub vendor_id: u16,
    /// USB product id.
    pub product_id: u16,
    /// Product string.
    pub product: String,
    /// Manufacturer string.
    pub manufacturer: String,
    /// HID usage page.
    pub usage_page: u16,
    /// Interface number, or a negative value when the OS has none.
    pub interface: i32,
}

/// A plausible LAMZU control interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// Opaque device path.
    pub path: CString,
    /// USB vendor id.
    pub vendor_id: u16,
    /// USB product id.
    pub product_id: u16,
    /// Product string.
    pub product: String,
    /// Manufacturer string.
    pub manufacturer: String,
    /// HID usage page.
    pub usage_page: u16,
    /// Interface number.
    pub interface: i32,
    /// Detected protocol.
    pub protocol: Protocol,
    /// Detected model.
    pub model: Option<String>,
}

impl DeviceInfo {
    /// `VVVV:PPPP` identifier.
    #[must_use]
    pub fn usb_id(&self) -> String {
        format!("{:04X}:{:04X}", self.vendor_id, self.product_id)
    }
}

/// Wired product ids: when a cable and a dongle are both present the wired
/// interface is polled first because it reports the live charging state.
const WIRED_IDS: [(u16, u16); 3] = [(0x3554, 0xF50F), (0x373E, 0x001C), (0x37B0, 0x0009)];

/// Models that can be identified by `VID:PID` even through a dongle that calls
/// itself `Receiver`.
const MODEL_BY_ID: [(u16, u16, &str); 10] = [
    (0x373E, 0x001E, "Maya X"),
    (0x373E, 0x001C, "Maya X"),
    (0x37B0, 0x0010, "Inca"),
    (0x37B0, 0x0009, "Inca"),
    (0x37B0, 0x000D, "Paro"),
    (0x37B0, 0x0001, "Paro"),
    (0x37B0, 0x0007, "Paro"),
    (0x37B0, 0x000E, "Paro"),
    (0x37B0, 0x0015, "Maya Champion"),
    (0x37B0, 0x0011, "Maya Champion"),
];

/// True when the device belongs to LAMZU.
#[must_use]
pub fn is_lamzu(device: &RawDevice) -> bool {
    if LAMZU_VENDOR_IDS.contains(&device.vendor_id) {
        return true;
    }
    format!("{} {}", device.product, device.manufacturer)
        .to_ascii_uppercase()
        .contains("LAMZU")
}

fn title_case(name: &str) -> String {
    name.split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => {
                    first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn strip_mouse_suffixes(name: &str) -> &str {
    let lower = name.to_ascii_lowercase();
    for suffix in [" gaming mouse", " wireless mouse", " wireless", " mouse"] {
        if lower.ends_with(suffix) {
            return name[..name.len() - suffix.len()].trim();
        }
    }
    name
}

fn receiver_like(product: &str) -> bool {
    let name = strip_mouse_suffixes(product.trim()).to_ascii_lowercase();
    ["receiver", "dongle", "wireless", "1k", "4k", "8k"]
        .iter()
        .any(|keyword| name.contains(keyword))
}

/// Extract a model name from a product string, if it is not a dongle.
#[must_use]
pub fn model_from_product(product: &str) -> Option<String> {
    if product.trim().is_empty() {
        return None;
    }
    let mut name = product.trim();
    if name.to_ascii_uppercase().starts_with("LAMZU") {
        name = name[5..].trim_matches([' ', '-', '_']);
    }
    let name = strip_mouse_suffixes(name).trim();
    if name.is_empty() || receiver_like(name) {
        return None;
    }
    Some(title_case(name))
}

/// Preferred model name for a device: the `VID:PID` table first, then the
/// product string.
#[must_use]
pub fn model_for(vendor_id: u16, product_id: u16, product: &str) -> Option<String> {
    MODEL_BY_ID
        .iter()
        .find(|(vid, pid, _)| *vid == vendor_id && *pid == product_id)
        .map(|(_, _, model)| (*model).to_owned())
        .or_else(|| model_from_product(product))
}

fn rank(device: &DeviceInfo) -> (u8, u8, i32) {
    let is_wired = WIRED_IDS.contains(&(device.vendor_id, device.product_id));
    let is_receiver = !is_wired && receiver_like(&device.product);
    let position = if is_wired {
        0
    } else if is_receiver {
        2
    } else {
        1
    };
    let protocol = match device.protocol {
        Protocol::Compx => 0,
        Protocol::Aurora => 1,
    };
    (position, protocol, device.interface)
}

/// Turn raw HID interfaces into ranked control-interface candidates.
///
/// The result contains only LAMZU interfaces with a known control usage page,
/// with the wired interface first, then plain interfaces, then dongles.
#[must_use]
pub fn select_candidates(raw: &[RawDevice]) -> Vec<DeviceInfo> {
    let mut seen: HashSet<&[u8]> = HashSet::new();
    let mut candidates = Vec::new();

    for device in raw {
        if !is_lamzu(device) || device.path.is_empty() {
            continue;
        }
        let protocol = match device.usage_page {
            USAGE_PAGE_COMPX => Protocol::Compx,
            USAGE_PAGE_AURORA => Protocol::Aurora,
            _ => continue,
        };
        if !seen.insert(device.path.as_slice()) {
            continue;
        }
        let Ok(path) = CString::new(device.path.clone()) else {
            continue;
        };
        candidates.push(DeviceInfo {
            path,
            vendor_id: device.vendor_id,
            product_id: device.product_id,
            product: device.product.clone(),
            manufacturer: device.manufacturer.clone(),
            usage_page: device.usage_page,
            interface: device.interface,
            protocol,
            model: model_for(device.vendor_id, device.product_id, &device.product),
        });
    }

    // Aurora's control interface is interface 2; when it exists, ignore the
    // other interfaces that share the same usage page.
    let has_aurora_control = candidates
        .iter()
        .any(|item| item.protocol == Protocol::Aurora && item.interface == AURORA_INTERFACE);
    if has_aurora_control {
        candidates
            .retain(|item| item.protocol != Protocol::Aurora || item.interface == AURORA_INTERFACE);
    }

    candidates.sort_by_key(rank);
    candidates
}

/// Product ids of the LAMZU devices that are currently present.
#[must_use]
pub fn present_product_ids(port: &dyn HidPort) -> Vec<(u16, u16)> {
    let devices = port.devices().unwrap_or_default();
    let mut ids: Vec<(u16, u16)> = devices
        .iter()
        .filter(|device| is_lamzu(device))
        .map(|device| (device.vendor_id, device.product_id))
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

fn parse_report(protocol: Protocol, report: &[u8]) -> Option<BatteryReading> {
    match protocol {
        Protocol::Compx => compx::parse(report),
        Protocol::Aurora => aurora::parse(report),
    }
}

/// Read the battery from the first Lamzu interface that answers.
///
/// Returns `None` when the mouse is asleep or no control interface responds.
/// A charging or full answer wins over a discharging one, because it is always
/// the fresher state when a cable and a dongle are connected together.
#[must_use]
pub fn read_battery(port: &dyn HidPort, timeout: Duration) -> Option<BatteryReading> {
    let devices = port.devices().ok()?;
    let candidates = select_candidates(&devices);
    let mut first: Option<BatteryReading> = None;

    for candidate in &candidates {
        let reports = port.exchange(candidate.path.as_c_str(), candidate.protocol, timeout);
        let Some(mut reading) = reports
            .iter()
            .find_map(|report| parse_report(candidate.protocol, report))
        else {
            log::debug!(
                "no reply from {} iface {} ({})",
                candidate.usb_id(),
                candidate.interface,
                candidate.protocol
            );
            continue;
        };
        reading.model.clone_from(&candidate.model);
        reading.protocol = candidate.protocol;
        reading.product = (!candidate.product.is_empty()).then(|| candidate.product.clone());
        if reading.charging() || reading.full() {
            return Some(reading);
        }
        if first.is_none() {
            first = Some(reading);
        }
    }
    first
}

/// Send the read-only battery request and return every raw report.
#[must_use]
pub fn probe_reports(port: &dyn HidPort, device: &DeviceInfo, timeout: Duration) -> Vec<Vec<u8>> {
    port.probe(device.path.as_c_str(), device.protocol, timeout)
}

/// Number of HID interfaces that belong to LAMZU.
#[must_use]
pub fn lamzu_interface_count(raw: &[RawDevice]) -> usize {
    raw.iter().filter(|device| is_lamzu(device)).count()
}

/// Path for diagnostics output.
#[must_use]
pub fn path_text(path: &CStr) -> String {
    path.to_string_lossy().into_owned()
}
