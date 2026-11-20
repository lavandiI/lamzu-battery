//! Pure decision logic: poll cadence, notification hysteresis and tray labels.
//!
//! Everything in this module is deterministic and free of I/O, which keeps the
//! interesting behaviour testable without threads, hardware or a desktop.

use std::time::Duration;

use crate::i18n::{self, Key, Language};
use crate::icons::{self, Rgba};
use crate::protocol::{BatteryReading, ChargeState, Percent};

/// Next poll interval for the current state.
#[must_use]
pub fn poll_interval(last: Option<&BatteryReading>, online: bool) -> Duration {
    match last {
        Some(reading) if online && reading.charging() => crate::POLL_INTERVAL_CHARGING,
        Some(_) if online => crate::POLL_INTERVAL,
        _ => crate::POLL_INTERVAL_OFFLINE,
    }
}

/// Notification that should be shown, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationKind {
    /// Battery dropped to the low threshold.
    Low,
    /// Battery reached 100%.
    Full,
}

/// Hysteresis state for the low/full notifications.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Notifications {
    low: bool,
    full: bool,
}

const LOW_ARM: Percent = Percent::saturating(15);
const LOW_REARM: Percent = Percent::saturating(20);
const FULL_HYSTERESIS: Percent = Percent::saturating(95);

impl Notifications {
    /// Decide whether a notification is due for this reading.
    ///
    /// `tray_ready` mirrors the Python behaviour of dropping notifications that
    /// would arrive before the tray icon exists; the next poll retries.
    pub fn observe(
        &mut self,
        reading: &BatteryReading,
        tray_ready: bool,
    ) -> Option<NotificationKind> {
        if reading.charging() {
            self.low = false;
            self.full = false;
            return None;
        }
        if reading.full() {
            if !self.full && tray_ready {
                self.full = true;
                return Some(NotificationKind::Full);
            }
            return None;
        }
        // Re-plugging the cable at 99% must not repeat the "charged" balloon.
        if reading.percent < FULL_HYSTERESIS {
            self.full = false;
        }
        if reading.percent <= LOW_ARM && !self.low && tray_ready {
            self.low = true;
            return Some(NotificationKind::Low);
        }
        if reading.percent > LOW_REARM {
            self.low = false;
        }
        None
    }

    /// Record whether the notification was delivered, so a failure is retried.
    pub fn delivered(&mut self, kind: NotificationKind, delivered: bool) {
        if !delivered {
            match kind {
                NotificationKind::Low => self.low = false,
                NotificationKind::Full => self.full = false,
            }
        }
    }
}

/// Delay until the next update check, together with the new failure counter.
#[must_use]
pub fn next_update_delay(success: bool, failures: u32) -> (u32, Duration) {
    if success {
        return (0, crate::UPDATE_CHECK_INTERVAL);
    }
    let index = usize::try_from(failures).unwrap_or(usize::MAX);
    let delay = crate::UPDATE_RETRY_DELAYS[index.min(crate::UPDATE_RETRY_DELAYS.len() - 1)];
    (failures.saturating_add(1), delay)
}

/// Model-aware display name shown in the menu and tooltip.
#[must_use]
pub fn display_name(last: Option<&BatteryReading>) -> String {
    match last.and_then(|reading| reading.model.as_deref()) {
        Some(model) => format!("LAMZU {model}"),
        None => crate::APP_NAME.to_owned(),
    }
}

/// Second menu line: percentage and state, or the age of the stale value.
#[must_use]
pub fn status_line(
    language: Language,
    last: Option<&BatteryReading>,
    online: bool,
    age: Duration,
) -> String {
    let Some(reading) = last else {
        return i18n::tr(language, Key::StatusNoMouse).to_owned();
    };
    if online {
        let state = i18n::state_text(language, reading.state);
        return format!("{}% · {state}", reading.percent);
    }
    let age = i18n::human_age(language, age.as_secs());
    i18n::tr_args(
        language,
        Key::StatusOffline,
        &[("percent", &reading.percent.to_string()), ("age", &age)],
    )
}

/// Tooltip text for the tray icon.
#[must_use]
pub fn tooltip(
    language: Language,
    name: &str,
    last: Option<&BatteryReading>,
    online: bool,
    age: Duration,
) -> String {
    let Some(reading) = last else {
        return i18n::tr_args(language, Key::TooltipNoConnection, &[("name", name)]);
    };
    if online {
        let state = i18n::state_text(language, reading.state);
        return format!("{name}: {}% ({state})", reading.percent);
    }
    let age = i18n::human_age(language, age.as_secs());
    i18n::tr_args(
        language,
        Key::TooltipOffline,
        &[
            ("name", name),
            ("percent", &reading.percent.to_string()),
            ("age", &age),
        ],
    )
}

/// Digits and colour of the tray icon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IconView {
    /// Text drawn on the icon.
    pub text: String,
    /// Colour of the text.
    pub color: Rgba,
}

/// Pick the icon contents for the current state.
#[must_use]
pub fn icon_view(
    last: Option<&BatteryReading>,
    age: Duration,
    online: bool,
    light_taskbar: bool,
) -> IconView {
    let palette = icons::palette(light_taskbar);
    let Some(reading) = last else {
        return IconView {
            text: "?".to_owned(),
            color: palette.stale,
        };
    };
    if online {
        return IconView {
            text: reading.percent.to_string(),
            color: icons::battery_color(
                reading.percent,
                reading.charging(),
                reading.full(),
                light_taskbar,
            ),
        };
    }
    IconView {
        text: if age < crate::STALE_READING {
            reading.percent.to_string()
        } else {
            "?".to_owned()
        },
        color: palette.stale,
    }
}

/// True when the reading is fresh enough to be shown normally.
#[must_use]
pub fn is_online(reading: Option<&BatteryReading>) -> bool {
    reading.is_some()
}

/// State name for logs.
#[must_use]
pub fn state_name(state: ChargeState) -> &'static str {
    match state {
        ChargeState::Discharging => "discharging",
        ChargeState::Charging => "charging",
        ChargeState::Full => "full",
        ChargeState::Other(_) => "unknown",
    }
}
