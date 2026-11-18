#!/usr/bin/env python3
"""LAMZU diagnostics: list HID interfaces and read the battery.

Usage:  python diagnose.py [--lang ru|en] [--raw]

Works with any Lamzu model: Compx (usage page 0xFF02) and Aurora
(usage page 0xFFFF, interface 2). The exit code is 1 when no interface is
found or the mouse does not answer, so scripts can tell a working setup from
a broken one.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import hid

import i18n
import lamzu
from lamzu import (
    BatteryReading,
    discover_devices,
    read_battery,
    voltage_to_percent,
)
from updater import executable_path
from version import APP_VERSION


def _print_devices(infos: list[dict], controls: dict) -> int:
    """Print every Lamzu HID interface and return how many were found."""
    found = 0
    for info in infos:
        if not lamzu.is_lamzu(info):
            continue
        found += 1
        vendor_id = lamzu.as_int(info.get("vendor_id"))
        product_id = lamzu.as_int(info.get("product_id"))
        product = str(info.get("product_string") or "")
        control = controls.get(info.get("path"))
        marker = ""
        if control is not None:
            model = control.model or i18n.tr("diagnose.model_unknown")
            marker = i18n.tr(
                "diagnose.control", protocol=control.protocol, model=model
            )
        print(
            f"  {vendor_id:04X}:{product_id:04X}  "
            f"iface {info.get('interface_number')}  "
            f"usage_page 0x{lamzu.as_int(info.get('usage_page')):04X}  "
            f"{product!r}{marker}"
        )
    return found


def _print_raw(controls: dict) -> None:
    """Print the raw replies of every control interface."""
    if not controls:
        print(i18n.tr("diagnose.raw_none"))
        return
    for control in controls.values():
        print(
            f"  {control.usb_id}  iface {control.interface}  "
            f"({control.protocol})"
        )
        reports = lamzu.probe_reports(control)
        if not reports:
            print(i18n.tr("diagnose.raw_none"))
            continue
        for report in reports:
            print("    " + " ".join(f"{byte:02x}" for byte in report))


def _print_reading(reading: BatteryReading) -> None:
    model = f"LAMZU {reading.model}" if reading.model else "LAMZU"
    print(
        i18n.tr(
            "diagnose.battery",
            percent=reading.percent,
            state=i18n.state_text(reading.state, reading.percent),
            model=model,
            protocol=reading.protocol_text,
        )
    )
    if reading.raw_percent is None:
        return
    line = i18n.tr("diagnose.raw_level", percent=reading.raw_percent)
    if reading.voltage_mv is not None:
        calculated = voltage_to_percent(reading.voltage_mv)
        line += "; " + i18n.tr(
            "diagnose.voltage",
            voltage=reading.voltage_mv,
            percent=calculated if calculated is not None else "—",
        )
    line += "; " + i18n.tr("diagnose.shown", percent=reading.percent)
    print("  " + line)


def main() -> int:
    parser = argparse.ArgumentParser(description="LAMZU diagnostics")
    parser.add_argument(
        "--lang",
        choices=i18n.LANGUAGES,
        default=None,
        help=i18n.translate("cli.lang_help", i18n.detect_system_language()),
    )
    parser.add_argument(
        "--raw",
        action="store_true",
        help=i18n.translate("cli.raw_help", i18n.detect_system_language()),
    )
    args = parser.parse_args()
    i18n.set_language(args.lang or i18n.detect_system_language())

    print(i18n.tr("diagnose.version", version=APP_VERSION))
    executable = executable_path()
    print(
        i18n.tr(
            "diagnose.executable",
            path=executable or Path(__file__).resolve(),
        )
    )

    try:
        infos = hid.enumerate()
    except Exception as exc:
        print(i18n.tr("diagnose.hid_error", error=exc))
        infos = []

    print(i18n.tr("diagnose.header"))
    controls = {item.path: item for item in discover_devices()}
    found = _print_devices(infos, controls)
    if not found:
        print(i18n.tr("diagnose.not_found"))

    if args.raw:
        print()
        print(i18n.tr("diagnose.raw_header"))
        _print_raw(controls)

    print()
    reading = read_battery()
    if reading is None:
        print(i18n.tr("diagnose.no_response"))
    else:
        _print_reading(reading)

    return 0 if found and reading is not None else 1


if __name__ == "__main__":
    sys.exit(main())
