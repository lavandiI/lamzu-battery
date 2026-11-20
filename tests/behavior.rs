//! Poll cadence, notification hysteresis, labels and palette contrast.

use std::time::Duration;

use lamzu_battery::core::{
    self, NotificationKind, Notifications, display_name, icon_view, next_update_delay,
    poll_interval, status_line, tooltip,
};
use lamzu_battery::i18n::Language;
use lamzu_battery::icons::{PALETTE_DARK, PALETTE_LIGHT, Rgba, battery_color, palette};
use lamzu_battery::protocol::{BatteryReading, ChargeState, Percent, Protocol};

fn reading(percent: u8, state: ChargeState) -> BatteryReading {
    BatteryReading {
        percent: Percent::new(percent).expect("valid percent"),
        state,
        model: Some("Thorn".to_owned()),
        protocol: Protocol::Compx,
        product: Some("LAMZU THORN".to_owned()),
        voltage_mv: None,
        raw_percent: None,
    }
}

#[test]
fn poll_intervals_follow_the_state() {
    assert_eq!(poll_interval(None, false), Duration::from_secs(20));
    assert_eq!(
        poll_interval(Some(&reading(50, ChargeState::Discharging)), false),
        Duration::from_secs(20)
    );
    assert_eq!(
        poll_interval(Some(&reading(50, ChargeState::Discharging)), true),
        Duration::from_secs(300)
    );
    assert_eq!(
        poll_interval(Some(&reading(50, ChargeState::Charging)), true),
        Duration::from_secs(60)
    );
}

#[test]
fn update_backoff_sequence() {
    let mut failures = 0;
    let mut delays = Vec::new();
    for _ in 0..6 {
        let (next, delay) = next_update_delay(false, failures);
        failures = next;
        delays.push(delay.as_secs());
    }
    assert_eq!(delays, vec![60, 300, 900, 3600, 21_600, 21_600]);
    assert_eq!(failures, 6);
    assert_eq!(
        next_update_delay(true, 5),
        (0, Duration::from_secs(6 * 60 * 60))
    );
}

#[test]
fn low_battery_is_retried_until_the_tray_is_ready() {
    let mut notifications = Notifications::default();
    let low = reading(10, ChargeState::Discharging);
    assert_eq!(notifications.observe(&low, false), None);
    assert_eq!(
        notifications.observe(&low, true),
        Some(NotificationKind::Low)
    );
    assert_eq!(notifications.observe(&low, true), None);
}

#[test]
fn failed_notification_is_retried() {
    let mut notifications = Notifications::default();
    let low = reading(10, ChargeState::Discharging);
    assert_eq!(
        notifications.observe(&low, true),
        Some(NotificationKind::Low)
    );
    notifications.delivered(NotificationKind::Low, false);
    assert_eq!(
        notifications.observe(&low, true),
        Some(NotificationKind::Low)
    );
}

#[test]
fn full_notification_has_hysteresis() {
    let mut notifications = Notifications::default();
    let full = reading(100, ChargeState::Full);
    assert_eq!(
        notifications.observe(&full, true),
        Some(NotificationKind::Full)
    );
    // Unplugging and replugging at 99% must not repeat the balloon.
    assert_eq!(
        notifications.observe(&reading(99, ChargeState::Discharging), true),
        None
    );
    assert_eq!(
        notifications.observe(&reading(99, ChargeState::Full), true),
        None
    );
    // A real discharge below 95% re-arms it.
    assert_eq!(
        notifications.observe(&reading(90, ChargeState::Discharging), true),
        None
    );
    assert_eq!(
        notifications.observe(&reading(100, ChargeState::Full), true),
        Some(NotificationKind::Full)
    );
}

#[test]
fn charging_resets_the_notification_state() {
    let mut notifications = Notifications::default();
    assert_eq!(
        notifications.observe(&reading(10, ChargeState::Discharging), true),
        Some(NotificationKind::Low)
    );
    assert_eq!(
        notifications.observe(&reading(10, ChargeState::Charging), true),
        None
    );
    assert_eq!(
        notifications.observe(&reading(10, ChargeState::Discharging), true),
        Some(NotificationKind::Low)
    );
}

#[test]
fn labels_use_the_model_name() {
    assert_eq!(
        display_name(Some(&reading(50, ChargeState::Discharging))),
        "LAMZU Thorn"
    );
    assert_eq!(display_name(None), "LAMZU Battery");
    assert_eq!(
        status_line(Language::En, None, false, Duration::ZERO),
        "No mouse connection"
    );
    assert_eq!(
        status_line(
            Language::En,
            Some(&reading(42, ChargeState::Discharging)),
            true,
            Duration::ZERO
        ),
        "42% · discharging"
    );
    assert_eq!(
        status_line(
            Language::En,
            Some(&reading(42, ChargeState::Discharging)),
            false,
            Duration::from_secs(300)
        ),
        "42% · no connection (5 min)"
    );
    assert_eq!(
        tooltip(
            Language::En,
            "LAMZU Thorn",
            Some(&reading(42, ChargeState::Discharging)),
            true,
            Duration::ZERO
        ),
        "LAMZU Thorn: 42% (discharging)"
    );
    assert_eq!(
        tooltip(Language::En, "LAMZU", None, false, Duration::ZERO),
        "LAMZU: no connection — move the mouse"
    );
}

#[test]
fn icon_view_switches_between_fresh_and_stale_values() {
    let fresh = icon_view(
        Some(&reading(42, ChargeState::Discharging)),
        Duration::from_secs(10),
        true,
        false,
    );
    assert_eq!(fresh.text, "42");
    assert_eq!(fresh.color, PALETTE_DARK.normal);

    let stale = icon_view(
        Some(&reading(42, ChargeState::Discharging)),
        Duration::from_secs(600),
        false,
        false,
    );
    assert_eq!(stale.text, "42");
    assert_eq!(stale.color, PALETTE_DARK.stale);

    let expired = icon_view(
        Some(&reading(42, ChargeState::Discharging)),
        Duration::from_secs(1000),
        false,
        false,
    );
    assert_eq!(expired.text, "?");

    let none = icon_view(None, Duration::ZERO, false, false);
    assert_eq!(none.text, "?");
    assert_eq!(none.color, PALETTE_DARK.stale);
}

#[test]
fn battery_colour_thresholds() {
    assert_eq!(
        battery_color(Percent::new(0).expect("valid"), false, false, false),
        PALETTE_DARK.low
    );
    assert_eq!(
        battery_color(Percent::new(29).expect("valid"), false, false, false),
        PALETTE_DARK.low
    );
    assert_eq!(
        battery_color(Percent::new(30).expect("valid"), false, false, false),
        PALETTE_DARK.normal
    );
    assert_eq!(
        battery_color(Percent::new(100).expect("valid"), false, false, false),
        PALETTE_DARK.normal
    );
    assert_eq!(
        battery_color(Percent::new(10).expect("valid"), true, false, false),
        PALETTE_DARK.charge
    );
    assert_eq!(
        battery_color(Percent::new(100).expect("valid"), false, true, false),
        PALETTE_DARK.charge
    );
    assert_eq!(
        battery_color(Percent::new(50).expect("valid"), false, false, true),
        PALETTE_LIGHT.normal
    );
    assert_eq!(palette(false), PALETTE_DARK);
    assert_eq!(palette(true), PALETTE_LIGHT);
}

fn luminance(color: Rgba) -> f32 {
    fn channel(value: u8) -> f32 {
        let value = f32::from(value) / 255.0;
        if value <= 0.039_28 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * channel(color.0[0]) + 0.7152 * channel(color.0[1]) + 0.0722 * channel(color.0[2])
}

fn contrast(a: Rgba, b: Rgba) -> f32 {
    let (first, second) = (luminance(a), luminance(b));
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

#[test]
fn light_palette_is_readable_on_a_light_taskbar() {
    let taskbar = Rgba([243, 243, 243, 255]);
    for color in [
        PALETTE_LIGHT.normal,
        PALETTE_LIGHT.low,
        PALETTE_LIGHT.charge,
        PALETTE_LIGHT.stale,
    ] {
        assert!(contrast(color, taskbar) >= 4.5, "{color:?}");
    }
}

#[test]
fn dark_palette_is_readable_on_a_dark_taskbar() {
    let taskbar = Rgba([32, 32, 32, 255]);
    for color in [
        PALETTE_DARK.normal,
        PALETTE_DARK.low,
        PALETTE_DARK.charge,
        PALETTE_DARK.stale,
    ] {
        assert!(contrast(color, taskbar) >= 3.0, "{color:?}");
    }
}

#[test]
fn state_names_match_the_wire_values() {
    assert_eq!(core::state_name(ChargeState::Discharging), "discharging");
    assert_eq!(core::state_name(ChargeState::Charging), "charging");
    assert_eq!(core::state_name(ChargeState::Full), "full");
    assert_eq!(core::state_name(ChargeState::Other(9)), "unknown");
}
