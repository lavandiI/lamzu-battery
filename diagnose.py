#!/usr/bin/env python3
"""LAMZU diagnostics: list HID interfaces and read the battery.

Usage:  python diagnose.py [--lang ru|en]

Works with any Lamzu model: Compx (usage page 0xFF02) and Aurora
(usage page 0xFFFF, interface 2).
"""

from __future__ import annotations

import argparse

import hid

import i18n
from lamzu import LAMZU_VENDOR_IDS, discover_devices, read_battery


def main() -> None:
    parser = argparse.ArgumentParser(description="LAMZU diagnostics")
    parser.add_argument(
        "--lang",
        choices=i18n.LANGUAGES,
        default=None,
        help=i18n.translate("cli.lang_help", i18n.detect_system_language()),
    )
    args = parser.parse_args()
    i18n.set_language(args.lang or i18n.detect_system_language())

    print(i18n.tr("diagnose.header"))
    found = 0
    controls = {item.path: item for item in discover_devices()}

    for info in hid.enumerate():
        vendor_id = int(info.get("vendor_id") or 0)
        product_id = int(info.get("product_id") or 0)
        product = str(info.get("product_string") or "")
        manufacturer = str(info.get("manufacturer_string") or "")
        if (
            vendor_id not in LAMZU_VENDOR_IDS
            and "LAMZU" not in f"{product} {manufacturer}".upper()
        ):
            continue

        found += 1
        control = controls.get(info.get("path"))
        if control is not None:
            model = control.model or i18n.tr("diagnose.model_unknown")
            marker = i18n.tr(
                "diagnose.control", protocol=control.protocol, model=model
            )
        else:
            marker = ""
        print(
            f"  {vendor_id:04X}:{product_id:04X}  "
            f"iface {info.get('interface_number')}  "
            f"usage_page 0x{int(info.get('usage_page') or 0):04X}  "
            f"{product!r}{marker}"
        )

    if not found:
        print(i18n.tr("diagnose.not_found"))

    print()
    reading = read_battery()
    if reading is None:
        print(i18n.tr("diagnose.no_response"))
    else:
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


if __name__ == "__main__":
    main()
