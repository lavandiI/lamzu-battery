import app


def test_condensed_font_is_used_for_multi_digit_values():
    assert app._use_condensed("95")
    assert app._use_condensed("100")
    assert not app._use_condensed("7")
    assert not app._use_condensed("?")


def test_rendered_icon_fits_the_canvas():
    image = app.render_icon("100", (0, 0, 0, 255), 64)
    box = image.getbbox()

    assert box is not None
    assert box[0] >= 0 and box[1] >= 0
    assert box[2] <= 64 and box[3] <= 64


def test_two_digit_icon_uses_the_icon_height():
    # The digits must not be limited by the icon width any more: this was
    # about 40 px of a 64 px canvas before the condensed font was used.
    image = app.render_icon("95", (0, 0, 0, 255), 64)
    box = image.getbbox()

    assert box is not None
    assert box[3] - box[1] >= 45
