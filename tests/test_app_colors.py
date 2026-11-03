import app


def test_red_below_thirty_percent():
    assert app.battery_color(0, False, False) == app.COLOR_LOW
    assert app.battery_color(15, False, False) == app.COLOR_LOW
    assert app.battery_color(29, False, False) == app.COLOR_LOW


def test_blue_from_thirty_percent():
    assert app.battery_color(30, False, False) == app.COLOR_NORMAL
    assert app.battery_color(31, False, False) == app.COLOR_NORMAL
    assert app.battery_color(100, False, False) == app.COLOR_NORMAL


def test_charging_and_full_are_green():
    assert app.battery_color(10, True, False) == app.COLOR_CHARGE
    assert app.battery_color(99, True, False) == app.COLOR_CHARGE
    assert app.battery_color(100, False, True) == app.COLOR_CHARGE


def test_yellow_is_not_used():
    assert not hasattr(app, "COLOR_WARN")
