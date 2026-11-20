//! Tests that touch the live system: the network, the registry, notifications.
//!
//! They are ignored by default because they are not hermetic:
//!
//! ```bat
//! cargo test --test live -- --ignored --nocapture
//! ```

use std::path::Path;

use lamzu_battery::autostart;
use lamzu_battery::notify::{Notifier, ToastNotifier};
use lamzu_battery::updater::http::{UreqTransport, UrlPolicy};
use lamzu_battery::updater::{check_for_update, normalize_digest};
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};

/// Ask the real GitHub API. A missing newer release is a valid answer here.
#[test]
#[ignore = "requires network access to api.github.com"]
fn github_release_check_works() {
    let result = check_for_update(
        &UreqTransport,
        &UrlPolicy::github(),
        lamzu_battery::VERSION,
        lamzu_battery::API_URL,
    )
    .expect("the GitHub API must answer with a valid release");
    if let Some(info) = result {
        assert!(
            lamzu_battery::updater::is_newer(&info.version, lamzu_battery::VERSION),
            "reported version must be newer: {}",
            info.version
        );
        assert!(
            normalize_digest(Some(&info.digest)).is_some(),
            "digest must be a SHA-256"
        );
        assert!(info.asset_size > 0);
        assert!(UrlPolicy::github().trusted_asset(&info.asset_url));
        println!("live update: {} ({} bytes)", info.version, info.asset_size);
    } else {
        println!(
            "no newer release than {} (this is expected for the local build)",
            lamzu_battery::VERSION
        );
    }
}

/// Write and remove the real autostart value, restoring whatever was there.
#[test]
#[ignore = "writes HKCU\\...\\CurrentVersion\\Run temporarily"]
fn autostart_roundtrip() {
    let fake = Path::new(r"C:\Tools\LamzuBench.exe");
    let readable = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(lamzu_battery::RUN_KEY, KEY_READ | KEY_SET_VALUE)
        .ok();
    let previous: Option<String> = readable
        .as_ref()
        .and_then(|key| key.get_value::<String, _>(lamzu_battery::RUN_VALUE).ok());

    autostart::set(true, fake).expect("enable autostart");
    assert!(autostart::enabled(fake));
    assert!(
        autostart::enabled(Path::new(r"c:\tools\lamzubench.exe")),
        "comparison must be case-insensitive"
    );

    autostart::set(false, fake).expect("disable autostart");
    assert!(!autostart::enabled(fake));

    match previous {
        Some(value) => {
            let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
                .create_subkey(lamzu_battery::RUN_KEY)
                .expect("open Run key");
            key.set_value(lamzu_battery::RUN_VALUE, &value)
                .expect("restore autostart");
            println!("restored the previous autostart value");
        }
        None => {
            if let Some(key) = readable {
                let _ = key.delete_value(lamzu_battery::RUN_VALUE);
            }
        }
    }
}

/// A published release must be offered to the previous Python build (1.5.11).
///
/// This pins the update contract: a trusted asset URL, a SHA-256 digest and a
/// sane size, so a broken release is caught before users update.
#[test]
#[ignore = "network: checks the published release contract"]
fn published_release_is_offered_to_1_5_11() {
    let info = check_for_update(
        &UreqTransport,
        &UrlPolicy::github(),
        "1.5.11",
        lamzu_battery::API_URL,
    )
    .expect("the GitHub API must answer")
    .expect("a release newer than 1.5.11 must exist");

    assert!(
        lamzu_battery::updater::is_newer(&info.version, "1.5.11"),
        "offered version {} is not newer",
        info.version
    );
    assert!(
        info.asset_url.contains("/releases/download/"),
        "asset URL is not a release download: {}",
        info.asset_url
    );
    assert!(UrlPolicy::github().trusted_asset(&info.asset_url));
    assert!(
        normalize_digest(Some(&info.digest)).is_some(),
        "the release asset must carry a SHA-256 digest"
    );
    assert!(
        info.asset_size > 1_000_000 && info.asset_size <= lamzu_battery::MAX_UPDATE_BYTES,
        "unexpected asset size: {}",
        info.asset_size
    );
    println!(
        "offered {} ({} bytes, {})",
        info.version, info.asset_size, info.asset_url
    );
}

/// Show a real Windows toast.
#[test]
#[ignore = "shows a desktop notification"]
fn toast_is_delivered() {
    let exe = std::env::current_exe().expect("current exe");
    ToastNotifier::register_app_id(&exe);
    let notifier = ToastNotifier::new();
    assert!(
        notifier.notify(
            "LAMZU Battery",
            "Test notification from the Rust build (1.6.0)"
        ),
        "WinRT rejected the toast"
    );
}
