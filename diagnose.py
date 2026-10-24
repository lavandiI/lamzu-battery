#!/usr/bin/env python3
"""Диагностика LAMZU: показать HID-интерфейсы и прочитать заряд.

Запуск:  python diagnose.py

Работает с любыми моделями Lamzu: Compx (usage page 0xFF02) и
Aurora (usage page 0xFFFF, интерфейс 2).
"""

from __future__ import annotations

import hid

from lamzu import LAMZU_VENDOR_IDS, discover_devices, read_battery


def main() -> None:
    print("LAMZU HID-интерфейсы:")
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
            model = control.model or "модель не определена"
            marker = f"  <-- управляющий: {control.protocol}, {model}"
        else:
            marker = ""
        print(
            f"  {vendor_id:04X}:{product_id:04X}  "
            f"iface {info.get('interface_number')}  "
            f"usage_page 0x{int(info.get('usage_page') or 0):04X}  "
            f"{product!r}{marker}"
        )

    if not found:
        print("  не найдено — проверьте, что донгл или кабель подключён")

    print()
    reading = read_battery()
    if reading is None:
        print("Заряд: нет ответа. Пошевелите мышь и запустите ещё раз.")
    else:
        model = f"LAMZU {reading.model}" if reading.model else "LAMZU"
        print(
            f"Заряд: {reading.percent}% ({reading.state_text}), "
            f"{model}, {reading.protocol_text}"
        )


if __name__ == "__main__":
    main()
