import app

DARK_TASKBAR = (32, 32, 32)
LIGHT_TASKBAR = (243, 243, 243)


def _luminance(color):
    def channel(value):
        value /= 255
        if value <= 0.03928:
            return value / 12.92
        return ((value + 0.055) / 1.055) ** 2.4

    red, green, blue = color[:3]
    return (
        0.2126 * channel(red)
        + 0.7152 * channel(green)
        + 0.0722 * channel(blue)
    )


def _contrast(first, second):
    left, right = _luminance(first), _luminance(second)
    high, low = max(left, right), min(left, right)
    return (high + 0.05) / (low + 0.05)


def test_red_below_thirty_percent():
    assert app.battery_color(0, False, False) == app.PALETTE_DARK["low"]
    assert app.battery_color(15, False, False) == app.PALETTE_DARK["low"]
    assert app.battery_color(29, False, False) == app.PALETTE_DARK["low"]


def test_blue_from_thirty_percent():
    assert app.battery_color(30, False, False) == app.PALETTE_DARK["normal"]
    assert app.battery_color(31, False, False) == app.PALETTE_DARK["normal"]
    assert app.battery_color(100, False, False) == app.PALETTE_DARK["normal"]


def test_charging_and_full_are_green():
    assert app.battery_color(10, True, False) == app.PALETTE_DARK["charge"]
    assert app.battery_color(99, True, False) == app.PALETTE_DARK["charge"]
    assert app.battery_color(100, False, True) == app.PALETTE_DARK["charge"]


def test_light_palette_is_used_when_asked():
    assert app.battery_color(50, False, False, True) == (
        app.PALETTE_LIGHT["normal"]
    )
    assert app.battery_color(10, False, False, True) == (
        app.PALETTE_LIGHT["low"]
    )
    assert app.battery_color(50, True, False, True) == (
        app.PALETTE_LIGHT["charge"]
    )


def test_palette_switch():
    assert app.palette() is app.PALETTE_DARK
    assert app.palette(False) is app.PALETTE_DARK
    assert app.palette(True) is app.PALETTE_LIGHT


def test_yellow_is_not_used():
    assert not hasattr(app, "COLOR_WARN")


def test_light_palette_is_readable_on_a_light_taskbar():
    for name, color in app.PALETTE_LIGHT.items():
        assert _contrast(color, LIGHT_TASKBAR) >= 4.5, name


def test_dark_palette_is_readable_on_a_dark_taskbar():
    for name, color in app.PALETTE_DARK.items():
        assert _contrast(color, DARK_TASKBAR) >= 3.0, name


class _Key:
    def __enter__(self):
        return self

    def __exit__(self, *args):
        return False


def test_light_theme_is_detected(monkeypatch):
    monkeypatch.setattr(app.winreg, "OpenKey", lambda *args, **kwargs: _Key())
    monkeypatch.setattr(
        app.winreg, "QueryValueEx", lambda key, name: (1, 4)
    )

    assert app.taskbar_uses_light_theme()


def test_dark_theme_is_the_default(monkeypatch):
    monkeypatch.setattr(app.winreg, "OpenKey", lambda *args, **kwargs: _Key())

    def missing(key, name):
        raise FileNotFoundError("no such value")

    monkeypatch.setattr(app.winreg, "QueryValueEx", missing)

    assert not app.taskbar_uses_light_theme()


def test_invalid_theme_value_is_ignored(monkeypatch):
    monkeypatch.setattr(app.winreg, "OpenKey", lambda *args, **kwargs: _Key())
    monkeypatch.setattr(
        app.winreg, "QueryValueEx", lambda key, name: ("garbage", 4)
    )

    assert not app.taskbar_uses_light_theme()
