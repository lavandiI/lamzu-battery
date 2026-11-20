//! Command line parsing and cross-thread type guarantees.

use clap::Parser;

use lamzu_battery::cli::Cli;
use lamzu_battery::i18n::Language;
use lamzu_battery::protocol::{BatteryReading, Percent, Protocol};
use lamzu_battery::tray::{MenuAction, UiCommand, UiSnapshot};
use static_assertions::assert_impl_all;

#[test]
fn defaults_are_quiet() {
    let cli = Cli::try_parse_from(["LamzuBattery"]).expect("defaults");
    assert!(!cli.debug);
    assert!(!cli.diagnose);
    assert!(!cli.raw);
    assert!(cli.lang.is_none());
    assert!(!cli.wait_for_instance);
}

#[test]
fn diagnostics_flags_parse() {
    let cli = Cli::try_parse_from(["LamzuBattery", "--diagnose", "--raw", "--lang", "ru"])
        .expect("diagnose flags");
    assert!(cli.diagnose);
    assert!(cli.raw);
    assert_eq!(cli.language(), Some(Language::Ru));
}

#[test]
fn raw_requires_diagnose() {
    assert!(Cli::try_parse_from(["LamzuBattery", "--raw"]).is_err());
}

#[test]
fn hidden_wait_for_instance_is_accepted() {
    let cli = Cli::try_parse_from(["LamzuBattery", "--wait-for-instance", "--debug"])
        .expect("wait for instance");
    assert!(cli.wait_for_instance);
    assert!(cli.debug);
}

#[test]
fn language_values_map_to_the_enum() {
    let cli = Cli::try_parse_from(["LamzuBattery", "--diagnose", "--lang", "en"]).expect("lang");
    assert_eq!(cli.language(), Some(Language::En));
    let cli = Cli::try_parse_from(["LamzuBattery", "--diagnose", "--lang", "ru"]).expect("lang");
    assert_eq!(cli.language(), Some(Language::Ru));
    assert!(Cli::try_parse_from(["LamzuBattery", "--diagnose", "--lang", "de"]).is_err());
}

#[test]
fn channel_payloads_are_send_and_sync() {
    assert_impl_all!(UiSnapshot: Send, Sync, Clone);
    assert_impl_all!(UiCommand: Send, Clone);
    assert_impl_all!(MenuAction: Send, Copy);
    assert_impl_all!(BatteryReading: Send, Sync, Clone);
    assert_impl_all!(Percent: Send, Sync, Copy);
    assert_impl_all!(Protocol: Send, Sync, Copy);
    assert_impl_all!(Language: Send, Sync, Copy);
}
