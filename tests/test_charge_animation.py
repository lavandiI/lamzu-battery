import threading

import app
from lamzu import BatteryReading


def _reading(percent: int, state: int = 0) -> BatteryReading:
    return BatteryReading(percent=percent, state=state)


def _opaque_color(image):
    for y in range(image.height):
        for x in range(image.width):
            pixel = image.getpixel((x, y))
            if pixel[3] > 200:
                return pixel
    raise AssertionError("rendered icon has no opaque pixels")


def test_charge_animation_timing_contract():
    assert app.CHARGE_ANIMATION_INTERVAL == 10.0
    assert app.CHARGE_ANIMATION_FRAMES == 15
    assert app.CHARGE_ANIMATION_FPS == 10
    assert app.CHARGE_ANIMATION_FRAMES / app.CHARGE_ANIMATION_FPS < 10.0


def test_palette_selection():
    assert app.palette_for(False) is app.PALETTE_DARK
    assert app.palette_for(True) is app.PALETTE_LIGHT


def test_state_color_follows_percent_and_charge_state():
    palette = app.PALETTE_DARK

    assert app.state_color(_reading(15), palette) == palette.low
    assert app.state_color(_reading(16), palette) == palette.warn
    assert app.state_color(_reading(30), palette) == palette.warn
    assert app.state_color(_reading(31), palette) == palette.normal
    assert app.state_color(_reading(80, state=1), palette) == palette.charge
    assert app.state_color(_reading(100, state=1), palette) == palette.charge
    assert app.state_color(_reading(100, state=2), palette) == palette.charge


def test_charge_start_color_uses_state_palette():
    for palette in (app.PALETTE_DARK, app.PALETTE_LIGHT):
        assert app.charge_start_color(_reading(15), palette) == palette.low
        assert app.charge_start_color(_reading(25), palette) == palette.warn
        assert app.charge_start_color(_reading(80), palette) == palette.normal


def test_charge_frame_starts_and_ends_with_expected_colors():
    palette = app.PALETTE_LIGHT
    first = app.render_charge_frame("15", palette.low, palette.charge, 0.0)
    last = app.render_charge_frame("15", palette.low, palette.charge, 1.0)

    assert _opaque_color(first) == palette.low
    assert _opaque_color(last) == palette.charge


def test_charge_frame_keeps_icon_size_and_alpha():
    frame = app.render_charge_frame(
        "100", app.PALETTE_DARK.normal, app.PALETTE_DARK.charge, 0.5
    )

    assert frame.size == (64, 64)
    assert frame.mode == "RGBA"
    assert frame.getbbox() is not None


def test_charge_frame_uses_hsv_path_not_grey():
    # Красный -> зелёный через RGB даёт грязно-коричневый. Через HSV
    # в середине должен получиться насыщенный жёлто-оранжевый.
    frame = app.render_charge_frame(
        "15", app.PALETTE_DARK.low, app.PALETTE_DARK.charge, 0.5
    )
    mid = _opaque_color(frame)
    assert mid[0] > mid[2]
    assert mid[1] > mid[2]


def test_system_uses_light_theme_reads_registry(monkeypatch):
    class Key:
        def __enter__(self):
            return self

        def __exit__(self, *args):
            return False

    monkeypatch.setattr(app.winreg, "OpenKey", lambda *args: Key())
    monkeypatch.setattr(
        app.winreg, "QueryValueEx", lambda key, value: (1, 0)
    )

    assert app.system_uses_light_theme() is True

    monkeypatch.setattr(
        app.winreg, "QueryValueEx", lambda key, value: (0, 0)
    )
    assert app.system_uses_light_theme() is False


def test_system_uses_light_theme_defaults_to_dark(monkeypatch):
    def fail(*args):
        raise OSError("missing")

    monkeypatch.setattr(app.winreg, "OpenKey", fail)

    assert app.system_uses_light_theme() is False


def test_animation_repeats_while_charging(tmp_path, monkeypatch):
    class FakeIcon:
        def __init__(self, name, icon, title, menu=None):
            self.visible = True
            self._icon = icon
            self.title = title
            self.menu = menu
            self.count = 0
            self.updated = threading.Event()

        @property
        def icon(self):
            return self._icon

        @icon.setter
        def icon(self, value):
            self._icon = value
            self.count += 1
            if self.count >= 6:
                self.updated.set()

        def update_menu(self):
            pass

        def notify(self, *args, **kwargs):
            pass

        def stop(self):
            self.visible = False

    monkeypatch.setattr(app.pystray, "Icon", FakeIcon)
    monkeypatch.setattr(app, "DATA_DIR", tmp_path)
    monkeypatch.setattr(app, "STATUS_FILE", tmp_path / "status.json")
    monkeypatch.setattr(app, "SETTINGS_FILE", tmp_path / "settings.json")
    monkeypatch.setattr(
        app, "read_battery", lambda: BatteryReading(percent=80, state=1)
    )
    monkeypatch.setattr(app, "CHARGE_ANIMATION_FRAMES", 3)
    monkeypatch.setattr(app, "CHARGE_ANIMATION_FPS", 100)
    monkeypatch.setattr(app, "CHARGE_ANIMATION_INTERVAL", 0.05)

    tray = app.BatteryTray()
    tray._poll()

    assert tray._charge_anim_thread is not None
    assert tray._charge_anim_thread.is_alive()
    assert tray._icon.updated.wait(1.0)

    monkeypatch.setattr(
        app, "read_battery", lambda: BatteryReading(percent=42, state=0)
    )
    tray._poll()

    tray._charge_anim_thread.join(timeout=1)
    assert not tray._charge_anim_thread.is_alive()
    assert tray._icon.count >= 6

    tray._stop.set()
    tray._cancel_charge_animation()

