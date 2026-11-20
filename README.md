# LAMZU Battery

<img src="docs/icon.png" width="96" align="right" alt="LAMZU Battery">

[![release](https://img.shields.io/github/v/release/lavandiI/lamzu-battery?label=release)](https://github.com/lavandiI/lamzu-battery/releases/latest)
[![tests](https://github.com/lavandiI/lamzu-battery/actions/workflows/tests.yml/badge.svg)](https://github.com/lavandiI/lamzu-battery/actions/workflows/tests.yml)
[![license](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![platform](https://img.shields.io/badge/platform-Windows%2010%2F11-0078D4)](#)

**English** | [Русский](README.ru.md)

A tiny battery indicator for LAMZU mice that lives in the Windows system tray.
It talks directly to the USB dongle or cable, requires no driver and does not
need the official LAMZU software to be running.

**[Download LamzuBattery.exe for Windows 10/11](https://github.com/lavandiI/lamzu-battery/releases/latest/download/LamzuBattery.exe)**

---

![Tray icon examples](docs/preview.png)

The digits are rendered at runtime from the system fonts: two- and three-digit
values use Bahnschrift's bold condensed instance, a single digit uses Segoe UI
Bold, and the palette follows the taskbar theme. `cargo run --example preview`
regenerates the image above.

## Features

- battery percentage as **large digits** directly on the tray icon;
- **precise percentage in 1% steps** for Compx while discharging, calculated from
  the battery voltage with the same curve as the official LAMZU software; while
  charging above 80% the device's 5% level is used, and 100% is shown only when
  the device itself reports raw level 100;
- bold colours without an outline: blue — normal, red — below 30%, dark green —
  charging, light blue — no fresh data; a light taskbar switches to darker shades
  with at least 4.5:1 contrast;
- **UI language switch** between English and Russian; the choice is remembered,
  the system language is used on first run;
- **automatic updates**: on startup and every six hours the app checks GitHub
  Releases, verifies the SHA-256 of the new executable, replaces the current file
  wherever it is located, checks that the new instance started correctly and
  keeps the rollback copy for about an hour; automatic installation can be
  disabled in the menu;
- tooltip with model, percentage and state; notifications on low battery and
  when fully charged;
- on Windows 11 22H2+ the icon is pinned to the taskbar automatically unless you
  hid it yourself; **Show icon on taskbar** restores it manually;
- start with Windows;
- cable/dongle plug and unplug is detected immediately;
- diagnostics: `LamzuBattery.exe --diagnose [--raw]`;
- no administrator rights, no driver, no telemetry, nothing installed
  system-wide.

## Quick start

1. Download `LamzuBattery.exe` from the
   [Releases](https://github.com/lavandiI/lamzu-battery/releases/latest) page.
2. Run it. Right-click the icon for the menu.
3. Optional: **Start with Windows**, **Show icon on taskbar**, **Language**.

The file is unsigned, so Windows SmartScreen may show a warning — normal for
small open-source projects.

## How it works

No driver is needed: the mouse or dongle exposes a regular HID interface with a
vendor-defined usage page. Only read-only battery queries are used.

### Compx

| Field | Value |
| --- | --- |
| VID:PID | `3554:F50D` (1K dongle), `3554:F50F` (cable/charging), `3554:F510` (4K/8K dongle, tentative) |
| HID usage page | `0xFF02` |
| Report ID | `8`, 17 bytes |
| Command | `4` — battery / charging status |
| Request | `08 04 00 00 00 00 00 00 00 00 00 00 00 00 00 00 49` |
| Response | `08 04 00 00 00 02 <level> <state> <voltage hi> <voltage lo> ... <checksum>` |
| State | `0` — discharging, `1` — charging, `2` — full |
| Voltage | bytes 8–9, big-endian millivolts |
| Level | byte 6, coarse percentage in 5% steps (fallback) |

Checksum: `(0x55 - sum(first 16 bytes)) & 0xFF`. The voltage is converted with
the official `BatteryParam` curve from LAMZU's `Config.ini` (3050 mV = 0%,
4110 mV = 100%, equal 5% steps), interpolated to 1%.

### Aurora

| Field | Value |
| --- | --- |
| VID:PID | `373E:001E/001C` (Maya X), `37B0:0010/0009` (Inca), Paro, Maya Champion |
| HID usage page | `0xFFFF`, interface `2` |
| Report | 65-byte feature report with report ID `0` |
| Current command | `0x83` at byte 6 |
| Legacy command | `0x8F` at byte 3 (fallback) |
| Response | tag `0xA1`; state and percent at bytes 7/8 (current) or 5/6 (legacy) |
| State | `0` — discharging, `1` — charging, `2` — full |

Settings, DPI, profiles, polling rate and firmware are never modified.

## Supported models

| Family | Examples | Protocol | Status |
| --- | --- | --- | --- |
| Compx | Thorn, Atlantis, Maya, 1K/4K/8K dongles `3554:F50D/F50F/F510` | report ID 8, usage page `0xFF02` | Thorn (cable and 1K dongle) tested; others use the common Compx protocol |
| Aurora | Maya X (`373E:001E/001C`), Inca (`37B0:0010/0009`), Paro, Maya Champion | feature report, usage page `0xFFFF`, interface 2 | based on the public protocol, not tested on hardware here |

The model comes from the product string (`LAMZU THORN` → `Thorn`) or from a table
of known VID:PIDs. A dongle that identifies itself as `Receiver` gives the
neutral `LAMZU Battery` name; the battery is still read.

## Polling

Updates every **5 minutes**. While charging it checks once a minute so it does
not miss 100%. If the mouse is asleep and does not answer, it retries every
20 seconds. Cable and dongle changes are watched separately every 5 seconds and
refresh the state immediately. **Refresh battery manually** in the menu polls at
any time.

## Where data is stored

Next to the executable:

```
data\status.json   — last state and update health marker
data\settings.json — selected UI language and update mode
data\app.log       — log (rotated at 256 KiB)
data\crash.log     — panic and native fault reports
```

If the executable directory is not writable (for example, `Program Files`),
`%LOCALAPPDATA%\LamzuBattery` is used instead.

## Autostart

Menu → **Start with Windows** writes the `LamzuBattery` value under
`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`. No administrator rights are
required. The entry written by 1.0–1.2 (`LamzuThornBattery`) is migrated
automatically.

## Updates

About 8 seconds after startup, then every six hours, the app asks the GitHub
Releases API for the latest version. If it is newer:

1. the folder and free space are checked before downloading;
2. `LamzuBattery.exe` is downloaded next to the running executable; progress is
   shown in the tooltip;
3. the SHA-256 published by GitHub is verified, together with the size and the
   `MZ` signature;
4. the current executable becomes `LamzuBattery.old.exe`, the new one takes its
   place, and the app restarts itself with `--wait-for-instance`;
5. the new instance must report a healthy tray within 30 seconds; if it does
   not, the previous version is restored and the error is written to the log.

The rollback copy is removed about an hour after a successful start (or on a
later start once it is older than a day). **Install updates automatically** can
be switched off; the app then only notifies and **Install X.Y.Z** appears in the
menu. A release without a SHA-256 `digest` in the API response is refused on
purpose.

## Performance

Version 1.6.0 is a complete rewrite in Rust; the previous Python/PyInstaller
build remains available in the git history. Measured on the same machine with
the same mouse ([method and caveats](docs/benchmark.md)):

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
enumerate HID interfaces every five seconds. Live checks on real hardware (menu
actions, `--wait-for-instance` hand-off, autostart, notifications, crash log,
GitHub API) are listed in [docs/verification.md](docs/verification.md).

## Building from source

Requirements: Windows 10/11 x64, Rust 1.95+ with the
`x86_64-pc-windows-msvc` toolchain, MSVC Build Tools (the HID library has a small
C component).

```bat
build.bat
```

The script runs `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo test`, then `cargo build --release --locked` and copies the result to
`dist\LamzuBattery.exe`.

Run from source:

```bat
cargo run --release
```

Tests (protocol parsers with property-based cases, updater with a local HTTP
server, i18n, settings, notifications, icon rendering, `unsafe` boundary):

```bat
cargo test
```

Tests that need the live system (network, registry, notifications) are ignored by
default:

```bat
cargo test --test live -- --ignored --nocapture
```

Regenerate the icon preview:

```bat
cargo run --example preview
```

## Diagnostics

```bat
LamzuBattery.exe --diagnose [--raw] [--lang ru|en]
```

Lists every LAMZU HID interface, the detected protocol and model, and the battery
reading; `--raw` also prints the raw reports. The exit code is 1 when no
interface is found or the mouse does not answer. Attach the output to an issue.

## Safety and reliability

- `#![deny(unsafe_code)]` at the crate root; `unsafe` lives only in `src/win/`,
  every block has a `// SAFETY:` proof, and `tests/unsafe_boundary.rs` fails if
  it appears anywhere else.
- Panics mean "stop": the release profile uses `panic = "abort"`, panics are
  written to `data\crash.log` and shown in a message box, and the autostart entry
  starts a fresh instance.
- Native faults are reported by a Win32 exception filter that formats into a
  fixed stack buffer and writes a pre-opened file: no allocation, no locks.
- 132 automated tests, `cargo clippy --all-targets -- -D warnings` and
  `cargo fmt --check` are clean.

## Troubleshooting

- **The icon shows `?`** — the mouse is off, asleep or out of range. Move it and
  use **Refresh battery manually**; the app also retries every 20 seconds.
- **The update left two icons or did not finish** — a failed update keeps the
  previous executable as `LamzuBattery.old.exe`. Close the app, rename that file
  to `LamzuBattery.exe` and start it again; details are in `data\app.log`.
- **The app does not start** — check `data\app.log` and `data\crash.log` next to
  the executable; the log also names the data folder it uses.
- **The model is not detected** — a dongle may report itself as `Receiver`, or an
  Aurora model may use a different report layout; the battery is still read.
  Attach `--diagnose --raw` output to an issue.
- **Settings are not remembered** — the app needs write access to its `data`
  folder; if it is read-only, settings live in `%LOCALAPPDATA%\LamzuBattery`.

## Limitations

- Windows only.
- Aurora is implemented from public descriptions and has not been tested on a
  real Aurora mouse.
- Compx 4K/8K dongle support (`F510`) is tentative.
- The update installation path is covered by tests with a local HTTP server and
  by the `--wait-for-instance` hand-off, but a live release-to-release update has
  not been exercised yet.
- The executable is unsigned; SmartScreen may warn.

## Alternatives

[HaloBattery](https://github.com/HeyOkay/HaloBattery),
[mouse_tray](https://github.com/Fan4Metal/mouse_tray) and
[lamzu-ctl](https://github.com/anany1702/lamzu-ctl).

## Contributing

Issues and pull requests are welcome. If your LAMZU model is not detected or does
not report its battery, include the output of `--diagnose --raw` and the mouse
model.

## License

[MIT](LICENSE). This project is not affiliated with LAMZU; it uses only publicly
documented read-only HID battery queries.
