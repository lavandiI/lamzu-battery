# LAMZU Battery — Rust implementation

A battery indicator for LAMZU mice in the Windows system tray, rewritten in Rust
from the Python original in the parent directory. The behaviour, the on-disk
formats and the release contract are intentionally identical, so the two builds
are interchangeable for users and for the self-updater.

![Tray icon examples](docs/preview.png)

The digits are rendered from the system fonts at runtime: two- and three-digit
values use Bahnschrift's bold condensed instance, a single digit uses Segoe UI
Bold, and the palette follows the taskbar theme. `cargo run --example preview`
regenerates the image above.

## Requirements

- Windows 10 or 11, x64.
- Rust 1.95 or newer with the `x86_64-pc-windows-msvc` toolchain (`rustup`).
- MSVC Build Tools (the HID library contains a small C component).

## Build

```bat
build.bat
```

The script runs `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo test`, then `cargo build --release --locked` and copies the result to
`dist\LamzuBattery.exe`.

Manual equivalent:

```bat
cargo build --release --locked
```

## Run

```bat
dist\LamzuBattery.exe
```

Useful switches:

| Switch | Meaning |
| --- | --- |
| `--debug` | verbose log |
| `--diagnose [--raw] [--lang ru\|en]` | print HID interfaces, raw reports and the battery reading, then exit |
| `--version`, `--help` | print and exit |
| `--wait-for-instance` | internal: wait up to 45 s for the previous instance to release the tray mutex |

## Tests

```bat
cargo test
```

The suite covers the protocol parsers (including property-based tests that feed
arbitrary bytes), the battery voltage curve, i18n and duration wording, settings
and status files, the notification hysteresis, the update preflight/download/
verify/replace/rollback paths (with a local HTTP server), interface ranking, tray
promotion decisions, icon rendering and the "no `unsafe` outside `src/win`" rule.

Tests that need the live system (network, registry, notifications) are ignored
by default:

```bat
cargo test --test live -- --ignored --nocapture
```

## Measured performance

[`docs/benchmark.md`](docs/benchmark.md) compares the shipped Python 1.5.11
executable with this build on the same machine and the same mouse:
| | Python 1.5.11 | Rust 1.6.0 |
| --- | ---: | ---: |
| Download size | 16.33 MiB | **2.61 MiB** |
| Temporary extraction | 31.4 MiB, 69 files | **none** |
| Ready after start | 700 ms | **301 ms** |
| First battery reading | 700 ms | **301 ms** |
| Peak private memory | 27.9 MiB | **5.6 MiB** |
| Second instance exit | 647 ms | **20 ms** |
| `--diagnose` | 200 ms | **96 ms** |
| Idle CPU (one core) | 0.2–0.3 % | 0.2–0.3 % |

Startup CPU drops by 4–7×; steady-state CPU is unchanged because both builds
enumerate HID interfaces every five seconds, and that behaviour is kept for
parity.

Live checks on real hardware — every menu action, the `--wait-for-instance`
hand-off, autostart, notifications, the crash log and the real GitHub API — are
listed in [`docs/verification.md`](docs/verification.md).

## Architecture

| Module | Responsibility |
| --- | --- |
| `main`, `cli` | entry point, argument parsing, console attachment for CLI modes |
| `app` | threads, menu actions, status file, update flow |
| `config` | data directory selection (`data` next to the exe, else `%LOCALAPPDATA%`, else `%TEMP%`) |
| `settings` | `settings.json` and `status.json` (merge, BOM tolerance, atomic writes) |
| `i18n` | English/Russian strings, placeholder rendering, localised durations |
| `icons`, `core` | palette, icon fitting/rendering, poll cadence, notification hysteresis, labels |
| `protocol` | Compx and Aurora parsers, voltage curve, interface discovery and ranking |
| `hid` | `hidapi` wrapper behind the `HidPort` trait |
| `tray` | tray icon and context menu on the main thread |
| `notify` | WinRT toast notifications and the AppUserModelId registration |
| `autostart`, `tray_visibility` | Run key and Windows 11 `NotifyIconSettings` promotion |
| `updater` | versions, preflight, verified download, replacement, rollback, health check |
| `diagnostics` | `--diagnose` output |
| `logging`, `crash` | rotating log and crash reporting |
| `win` | the only module allowed to contain `unsafe`: Win32 translation layer |

## Safety policy

- `#![deny(unsafe_code)]` at the crate root; `unsafe` lives only in `src/win/`,
  which carries `#[expect(unsafe_code, reason = …)]`.
- Every `unsafe` block has a `// SAFETY:` comment naming the Win32 contract it
  discharges; there are no public `unsafe fn`s, no `transmute`, no handwritten
  `unsafe impl Send/Sync` and no zeroed POD where validity is not guaranteed.
- Handles are stored as integers and closed exactly once; pointers never escape.
- The unhandled-exception filter formats into a fixed stack buffer and writes a
  pre-opened file: it does not allocate, lock or panic.
- `tests/unsafe_boundary.rs` fails the build if `unsafe` appears outside
  `src/win/`.

Trusted dependencies (the TCB) are `windows-sys`, `winreg`, `hidapi`,
`tray-icon`/`muda`, `swash`/`skrifa`, `ureq`, `serde`, `sha2` and
`tauri-winrt-notification`.

## Panic policy

Panics mean "stop the program": the release profile uses `panic = "abort"` and
worker loops do not swallow panics with `catch_unwind`. A panic is written to
`data\crash.log`, shown in a message box on the UI thread, and the process exits;
the autostart entry (or the user) starts a fresh instance. Recoverable failures
(HID, network, files, registry) are `Result`s and never panic.

## Data and compatibility

`data\status.json`, `data\settings.json`, `data\app.log` and `data\crash.log`
use the Python layout, including the `healthy`/`heartbeat_unix` fields read by
the updater's health check. `LamzuBattery.old.exe`, `.new.exe` and `.part` files
follow the same naming. Mutexes (`Local\LamzuBatteryTray`,
`Local\LamzuBatteryUpdateFiles`) and registry keys are unchanged, so only one of
the two builds can run at a time and the existing release asset
(`LamzuBattery.exe`) updates both directions.

The Rust build reports version `1.6.0`; a release tagged `v1.6.0` with the
`LamzuBattery.exe` asset needs the SHA-256 `digest` that GitHub publishes for the
updater to accept it.

## Intentional differences from the Python version

1. The menu is rebuilt only when its labels change (the Python build called
   `update_menu()` from a worker, which could destroy an open popup).
2. Notifications use WinRT toasts with a registered AppUserModelId instead of a
   legacy balloon (Windows 10/11 renders both as toasts).
3. Language entries are check items (muda has no radio item); the mutual
   exclusion behaves the same.
4. A product string containing "wireless" no longer demotes an interface to
   "dongle" after the `… Mouse` suffix is stripped (AUDIT H2).
5. No PyInstaller `_PYI_*` environment cleanup and no mandatory `taskkill /T`:
   the process is a plain executable, with `kill` and a taskkill fallback.
6. `--version` prints `LamzuBattery 1.6.0` (clap style) instead of `1.5.11`.
7. `diagnose.py` became `LamzuBattery.exe --diagnose`.
8. The executable carries the icon, version resource and a PerMonitorV2 manifest.

## Deliberately not adopted

- **Miri** cannot execute Win32 FFI, so it is not a gate; the unsafe surface is
  instead small, isolated and reviewed.
- **mimalloc** is unnecessary: there is no allocation hot path, and it would grow
  the binary and the antivirus surface.
- **`target-cpu=x86-64-v2`** would drop older Windows 10 machines; the tray app
  has no hot loop that would benefit.
- **Structured key/value logging** would break the documented `app.log` format.
- **`clippy::pedantic`** is enabled selectively through `#[expect]`; the full
  group produces mostly noise for the Win32 translation layer.
- **CI and release workflows** live at the repository root and still build the
  Python app; migrating them is a separate decision.

## Manual acceptance checklist

- [ ] `dist\LamzuBattery.exe` starts and shows the percentage in the tray.
- [ ] Tray menu: refresh, autostart toggle, data folder, taskbar pin, release
      page, update mode, check for updates, language switch, exit.
- [ ] Language switch updates every label and survives a restart.
- [ ] Tooltip shows model, percentage and state (or "no connection").
- [ ] A second instance exits immediately.
- [ ] `--diagnose` prints the interfaces and the battery reading.
- [ ] `data\app.log` contains the startup lines; `data\status.json` reports
      `healthy: true` a few seconds after start.
- [ ] Exiting writes `running: false`.

## Licence

MIT, like the original project. The icon is the original project asset.
