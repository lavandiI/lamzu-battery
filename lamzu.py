"""Универсальное чтение заряда мышей LAMZU через HID.

Поддерживаются два протокола, которые встречаются у разных моделей Lamzu:

* **Compx** — управляющий интерфейс с usage page ``0xFF02``, report ID 8,
  17-байтовые отчёты. Запрос ``08 04 ...``, процент в байте 6, состояние
  в байте 7 (0 — разряд, 1 — зарядка, 2 — заряжено).
* **Aurora** — интерфейс 2 с usage page ``0xFFFF``, 65-байтовый feature
  report с report ID 0. Новый вариант использует команду ``0x83``,
  старый — ``0x8F``. Ответ помечен ``0xA1``; состояние и процент лежат
  в байтах 7/8 (новый) или 5/6 (старый).

Устройство ищется не по конкретному PID, а по vendor ID Lamzu и по
управляющему usage page, поэтому новые модели подхватываются без правок
кода. Если Lamzu выпустит устройство с другим VID, оно всё равно будет
найдено по строке «LAMZU» в описании.

Мышь опрашивается только read-only командой. Настройки, DPI, профили и
прошивка не изменяются.
"""

from __future__ import annotations

import logging
import time
from dataclasses import dataclass, replace
from typing import Optional

import hid

# Известные VID Lamzu: старые Compx-устройства и новые Aurora-модели.
LAMZU_VENDOR_IDS = frozenset({0x3554, 0x373E, 0x37B0})

USAGE_PAGE_COMPX = 0xFF02
USAGE_PAGE_AURORA = 0xFFFF
AURORA_INTERFACE = 2

# Compx: 17 байт, report ID 8, команда 4.
COMPX_REPORT_ID = 0x08
COMPX_BATTERY_COMMAND = 0x04
COMPX_REQUEST = bytes.fromhex(
    "08 04 00 00 00 00 00 00 00 00 00 00 00 00 00 00 49"
)

# Aurora: 65 байт, report ID 0. Новый и старый варианты команды.
AURORA_REPORT_ID = 0x00
AURORA_REPLY_TAG = 0xA1
AURORA_BATTERY_COMMAND = 0x83
AURORA_LEGACY_COMMAND = 0x8F

# Модели, которые можно определить по VID:PID даже через донгл, когда
# в product string написано только «Receiver».
_MODEL_BY_ID: dict[tuple[int, int], str] = {
    (0x373E, 0x001E): "Maya X",
    (0x373E, 0x001C): "Maya X",
    (0x37B0, 0x0010): "Inca",
    (0x37B0, 0x0009): "Inca",
    (0x37B0, 0x000D): "Paro",
    (0x37B0, 0x0001): "Paro",
    (0x37B0, 0x0007): "Paro",
    (0x37B0, 0x000E): "Paro",
    (0x37B0, 0x0015): "Maya Champion",
    (0x37B0, 0x0011): "Maya Champion",
}

# Проводные PID: при одновременно подключённых кабеле и донгле проводной
# интерфейс опрашивается первым, потому что он показывает актуальную зарядку.
_WIRED_IDS = frozenset(
    {
        (0x3554, 0xF50F),
        (0x373E, 0x001C),
        (0x37B0, 0x0009),
    }
)

log = logging.getLogger(__name__)


@dataclass(frozen=True)
class BatteryReading:
    percent: int
    state: int
    model: Optional[str] = None
    protocol: str = ""
    product: Optional[str] = None

    @property
    def charging(self) -> bool:
        # На кабеле при 100 % некоторые модели продолжают отдавать state=1.
        return self.state == 1 and self.percent < 100

    @property
    def full(self) -> bool:
        return self.state == 2 or (self.state == 1 and self.percent >= 100)

    @property
    def state_text(self) -> str:
        if self.full:
            return "заряжена"
        return {
            0: "разряд",
            1: "заряжается",
            2: "заряжена",
        }.get(self.state, f"состояние {self.state}")

    @property
    def protocol_text(self) -> str:
        return {"compx": "Compx", "aurora": "Aurora"}.get(
            self.protocol, self.protocol or "unknown"
        )


@dataclass(frozen=True)
class DeviceInfo:
    path: bytes
    vendor_id: int
    product_id: int
    product: str
    manufacturer: str
    usage_page: int
    interface: Optional[int]
    protocol: str
    model: Optional[str]

    @property
    def usb_id(self) -> str:
        return f"{self.vendor_id:04X}:{self.product_id:04X}"


def _as_int(value) -> int:
    try:
        return int(value or 0)
    except (TypeError, ValueError):
        return 0


def _is_lamzu(info: dict) -> bool:
    if _as_int(info.get("vendor_id")) in LAMZU_VENDOR_IDS:
        return True
    text = " ".join(
        str(info.get(key) or "")
        for key in ("product_string", "manufacturer_string")
    ).upper()
    return "LAMZU" in text


def _is_receiver_name(product: str) -> bool:
    lowered = product.lower()
    return any(
        word in lowered
        for word in ("receiver", "dongle", "wireless", "1k", "4k", "8k")
    )


def _model_from_product(product: str) -> Optional[str]:
    """Вытащить название модели из product string, если это не донгл."""
    if not product:
        return None
    name = product.strip()
    if name.upper().startswith("LAMZU"):
        name = name[5:].strip(" -_")
    if not name or _is_receiver_name(name):
        return None
    lowered = name.lower()
    for suffix in (" gaming mouse", " wireless mouse", " mouse"):
        if lowered.endswith(suffix):
            name = name[: -len(suffix)].strip()
            break
    return name.title() if name else None


def _model_for(vendor_id: int, product_id: int, product: str) -> Optional[str]:
    return _MODEL_BY_ID.get((vendor_id, product_id)) or _model_from_product(product)


def discover_devices() -> list[DeviceInfo]:
    """Найти все управляющие HID-интерфейсы Lamzu.

    Возвращает кандидатов в порядке предпочтения: проводные интерфейсы
    раньше донглов, затем Compx/Aurora, затем по номеру интерфейса.
    """
    try:
        infos = hid.enumerate()
    except Exception as exc:
        log.debug("hid.enumerate failed: %s", exc)
        return []

    candidates: list[DeviceInfo] = []
    seen: set[bytes] = set()
    for info in infos:
        if not _is_lamzu(info):
            continue
        path = info.get("path")
        if not path or path in seen:
            continue
        usage_page = _as_int(info.get("usage_page"))
        if usage_page == USAGE_PAGE_COMPX:
            protocol = "compx"
        elif usage_page == USAGE_PAGE_AURORA:
            protocol = "aurora"
        else:
            continue

        seen.add(path)
        vendor_id = _as_int(info.get("vendor_id"))
        product_id = _as_int(info.get("product_id"))
        product = str(info.get("product_string") or "")
        interface = info.get("interface_number")
        if interface is not None:
            interface = int(interface)
        candidates.append(
            DeviceInfo(
                path=path,
                vendor_id=vendor_id,
                product_id=product_id,
                product=product,
                manufacturer=str(info.get("manufacturer_string") or ""),
                usage_page=usage_page,
                interface=interface,
                protocol=protocol,
                model=_model_for(vendor_id, product_id, product),
            )
        )

    # У Aurora управляющий интерфейс — именно 2. Если он есть, остальные
    # интерфейсы с тем же usage page не трогаем.
    aurora_control = [
        item
        for item in candidates
        if item.protocol == "aurora" and item.interface == AURORA_INTERFACE
    ]
    if aurora_control:
        candidates = [
            item
            for item in candidates
            if item.protocol != "aurora"
            or item.interface == AURORA_INTERFACE
        ]

    def rank(item: DeviceInfo) -> tuple[int, int, int]:
        wired = 0 if (item.vendor_id, item.product_id) in _WIRED_IDS else 1
        if _is_receiver_name(item.product):
            wired += 2
        protocol = 0 if item.protocol == "compx" else 1
        interface = item.interface if item.interface is not None else 99
        return (wired, protocol, interface)

    candidates.sort(key=rank)
    return candidates


def present_product_ids() -> frozenset[tuple[int, int]]:
    """Какие VID:PID Lamzu сейчас подключены (без запросов к мыши)."""
    try:
        infos = hid.enumerate()
    except Exception:
        return frozenset()
    return frozenset(
        (_as_int(info.get("vendor_id")), _as_int(info.get("product_id")))
        for info in infos
        if _is_lamzu(info)
    )


def _parse_compx(data: list[int]) -> Optional[BatteryReading]:
    """Разобрать ответ Compx, принимая оба варианта (с report ID и без)."""
    reading: Optional[BatteryReading] = None
    if len(data) >= 17 and data[0] == COMPX_REPORT_ID and data[1] == COMPX_BATTERY_COMMAND:
        level, state, checksum = data[6], data[7], data[16]
        expected = (0x55 - sum(data[:16])) & 0xFF
        if checksum != expected:
            log.debug(
                "bad Compx checksum: got 0x%02x, expected 0x%02x", checksum, expected
            )
            return None
        reading = BatteryReading(percent=level, state=state)
    elif len(data) >= 16 and data[0] == COMPX_REPORT_ID and data[1] == COMPX_BATTERY_COMMAND:
        # Ответ без контрольной суммы — встречается у некоторых ревизий.
        reading = BatteryReading(percent=data[6], state=data[7])
    elif len(data) >= 7 and data[0] == COMPX_BATTERY_COMMAND:
        # Некоторые сборки hidapi/драйвера отдают ответ без report ID.
        reading = BatteryReading(percent=data[5], state=data[6])
    else:
        return None

    if 0 <= reading.percent <= 100 and 0 <= reading.state <= 3:
        return reading

    log.debug("unexpected Compx payload: %s", data)
    return None


def _parse_aurora(data: list[int]) -> Optional[BatteryReading]:
    """Разобрать ответ Aurora (новый и старый протокол, с ID и без)."""
    reading: Optional[BatteryReading] = None

    if (
        len(data) >= 9
        and data[1] == AURORA_REPLY_TAG
        and data[4] == 0x02
        and data[6] == AURORA_BATTERY_COMMAND
    ):
        reading = BatteryReading(percent=data[8], state=data[7])
    elif (
        len(data) >= 8
        and data[0] == AURORA_REPLY_TAG
        and data[3] == 0x02
        and data[5] == AURORA_BATTERY_COMMAND
    ):
        reading = BatteryReading(percent=data[7], state=data[6])
    elif (
        len(data) >= 7
        and data[1] == AURORA_REPLY_TAG
        and data[2] == 0x02
        and data[3] == AURORA_LEGACY_COMMAND
    ):
        reading = BatteryReading(percent=data[6], state=data[5])
    elif (
        len(data) >= 6
        and data[0] == AURORA_REPLY_TAG
        and data[1] == 0x02
        and data[2] == AURORA_LEGACY_COMMAND
    ):
        reading = BatteryReading(percent=data[5], state=data[4])
    else:
        return None

    if 0 <= reading.percent <= 100 and 0 <= reading.state <= 3:
        return reading

    log.debug("unexpected Aurora payload: %s", data)
    return None


def _compx_request() -> bytes:
    return COMPX_REQUEST


def _aurora_request(new_protocol: bool) -> bytes:
    report = bytearray(65)
    report[0] = AURORA_REPORT_ID
    if new_protocol:
        report[3] = 0x02  # mouse
        report[4] = 0x02  # length
        report[6] = AURORA_BATTERY_COMMAND
    else:
        report[2] = 0x02  # mouse
        report[3] = AURORA_LEGACY_COMMAND
        report[4] = 0x01  # wireless receiver target
    return bytes(report)


def _read_compx(path: bytes, timeout: float) -> Optional[BatteryReading]:
    device = hid.device()
    try:
        device.open_path(path)
        if device.write(_compx_request()) != len(COMPX_REQUEST):
            return None

        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            data = device.read(64, 40)
            if data:
                reading = _parse_compx(list(data))
                if reading is not None:
                    return reading

        # Некоторые устройства отвечают не input-, а feature-отчётом.
        try:
            feature = device.get_feature_report(COMPX_REPORT_ID, 17)
        except Exception:
            feature = None
        if feature:
            return _parse_compx(list(feature))
    except Exception as exc:
        log.debug("Compx read failed on %r: %s", path, exc)
    finally:
        try:
            device.close()
        except Exception:
            pass
    return None


def _read_aurora(path: bytes, timeout: float) -> Optional[BatteryReading]:
    device = hid.device()
    try:
        device.open_path(path)
        per_revision = max(0.15, timeout / 2)
        for new_protocol in (True, False):
            request = _aurora_request(new_protocol)
            sent = False
            try:
                sent = device.send_feature_report(request) >= 0
            except Exception as exc:
                log.debug("Aurora feature send failed on %r: %s", path, exc)
            if not sent:
                # На некоторых сборках работает обычный output report.
                try:
                    device.write(request)
                    sent = True
                except Exception as exc:
                    log.debug("Aurora write failed on %r: %s", path, exc)

            if not sent:
                continue

            deadline = time.monotonic() + per_revision
            time.sleep(0.02)
            while time.monotonic() < deadline:
                try:
                    feature = device.get_feature_report(AURORA_REPORT_ID, 65)
                except Exception:
                    feature = None
                if feature:
                    reading = _parse_aurora(list(feature))
                    if reading is not None:
                        return reading

                data = device.read(65, 30)
                if data:
                    reading = _parse_aurora(list(data))
                    if reading is not None:
                        return reading
    except Exception as exc:
        log.debug("Aurora read failed on %r: %s", path, exc)
    finally:
        try:
            device.close()
        except Exception:
            pass
    return None


def read_battery(timeout: float = 0.5) -> Optional[BatteryReading]:
    """Опросить все найденные управляющие интерфейсы Lamzu.

    Возвращает None, если мышь спит или ни один интерфейс не ответил.
    Если одновременно подключены кабель и донгл, предпочтение отдаётся
    ответу «заряжается/заряжена» — он всегда актуальнее.
    """
    candidates = discover_devices()
    if not candidates:
        log.debug("no Lamzu control interface found")
        return None

    first: Optional[BatteryReading] = None
    for candidate in candidates:
        if candidate.protocol == "compx":
            reading = _read_compx(candidate.path, timeout)
        else:
            reading = _read_aurora(candidate.path, timeout)

        if reading is None:
            log.debug(
                "no reply from %s iface %s (%s)",
                candidate.usb_id,
                candidate.interface,
                candidate.protocol,
            )
            continue

        reading = replace(
            reading,
            model=candidate.model,
            protocol=candidate.protocol,
            product=candidate.product or None,
        )
        if reading.charging or reading.full:
            return reading
        if first is None:
            first = reading

    return first
