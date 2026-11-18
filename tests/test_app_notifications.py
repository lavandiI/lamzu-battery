import app
from lamzu import BatteryReading


class _Icon:
    def __init__(self, visible=True):
        self.visible = visible
        self.notifications = []

    def notify(self, message, title=None):
        self.notifications.append(message)


def _tray(visible=True):
    tray = object.__new__(app.BatteryTray)
    tray._icon = _Icon(visible)
    tray._low_notified = False
    tray._full_notified = False
    tray._display_name = lambda: "LAMZU"
    return tray


def test_notify_is_skipped_until_the_icon_is_ready():
    tray = _tray(visible=False)

    assert tray._notify("body", "title") is False
    assert tray._icon.notifications == []

    tray._icon.visible = True
    assert tray._notify("body", "title") is True
    assert tray._icon.notifications == ["body"]


def test_low_battery_is_retried_after_an_early_poll():
    tray = _tray(visible=False)

    tray._maybe_notify(BatteryReading(percent=10, state=0))
    assert not tray._low_notified
    assert tray._icon.notifications == []

    tray._icon.visible = True
    tray._maybe_notify(BatteryReading(percent=10, state=0))
    assert tray._low_notified
    assert len(tray._icon.notifications) == 1


def test_low_battery_is_reported_once():
    tray = _tray()

    tray._maybe_notify(BatteryReading(percent=10, state=0))
    tray._maybe_notify(BatteryReading(percent=10, state=0))

    assert len(tray._icon.notifications) == 1


def test_full_notification_has_hysteresis():
    tray = _tray()

    tray._maybe_notify(BatteryReading(percent=100, state=2))
    assert len(tray._icon.notifications) == 1

    # Unplug and plug back in at 99%: no second balloon.
    tray._maybe_notify(BatteryReading(percent=99, state=0))
    tray._maybe_notify(BatteryReading(percent=99, state=2))
    assert len(tray._icon.notifications) == 1

    # A real discharge below 95% re-arms the notification.
    tray._maybe_notify(BatteryReading(percent=90, state=0))
    tray._maybe_notify(BatteryReading(percent=100, state=2))
    assert len(tray._icon.notifications) == 2
