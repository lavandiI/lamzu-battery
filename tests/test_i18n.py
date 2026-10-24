import pytest

import i18n


@pytest.fixture(autouse=True)
def restore_language():
    previous = i18n.get_language()
    yield
    i18n.set_language(previous)


def test_state_text_russian():
    assert i18n.state_text(0, 55, "ru") == "разряд"
    assert i18n.state_text(1, 55, "ru") == "заряжается"
    assert i18n.state_text(1, 100, "ru") == "заряжена"
    assert i18n.state_text(2, 100, "ru") == "заряжена"
    assert i18n.state_text(9, 55, "ru") == "состояние 9"


def test_state_text_english():
    assert i18n.state_text(0, 55, "en") == "discharging"
    assert i18n.state_text(1, 55, "en") == "charging"
    assert i18n.state_text(1, 100, "en") == "full"
    assert i18n.state_text(2, 100, "en") == "full"
    assert i18n.state_text(9, 55, "en") == "state 9"


def test_current_language_and_formatting():
    assert i18n.set_language("ru") == "ru"
    assert i18n.tr("menu.language") == "Язык"
    assert i18n.tr("status.offline", percent=42, age="5 мин") == (
        "42% · нет связи (5 мин)"
    )

    assert i18n.set_language("en") == "en"
    assert i18n.tr("menu.language") == "Language"
    assert i18n.tr("status.offline", percent=42, age="5 min") == (
        "42% · no connection (5 min)"
    )


def test_human_age():
    assert i18n.human_age(5, "ru") == "только что"
    assert i18n.human_age(5 * 60, "ru") == "5 мин"
    assert i18n.human_age(2 * 3600 + 10 * 60, "ru") == "2 ч 10 мин"
    assert i18n.human_age(2 * 86400 + 3 * 3600, "ru") == "2 д 3 ч"

    assert i18n.human_age(5, "en") == "just now"
    assert i18n.human_age(5 * 60, "en") == "5 min"
    assert i18n.human_age(2 * 3600 + 10 * 60, "en") == "2 h 10 min"
    assert i18n.human_age(2 * 86400 + 3 * 3600, "en") == "2 d 3 h"


def test_unknown_language_falls_back_to_english():
    assert i18n.set_language("de") == "en"
    assert i18n.tr("menu.exit") == "Exit"
