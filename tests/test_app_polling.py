import app
from lamzu import BatteryReading


def _tray():
    tray = object.__new__(app.BatteryTray)
    tray._last = None
    tray._online = False
    return tray


def test_poll_intervals():
    tray = _tray()
    assert tray._interval() == app.POLL_INTERVAL_OFFLINE

    tray._last = BatteryReading(percent=50, state=0)
    tray._online = True
    assert app.POLL_INTERVAL == 5 * 60
    assert tray._interval() == 5 * 60

    tray._last = BatteryReading(percent=50, state=1)
    assert tray._interval() == app.POLL_INTERVAL_CHARGING
