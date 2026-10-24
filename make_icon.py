"""Сгенерировать icon.ico для exe и ярлыков."""

from pathlib import Path

from PIL import Image, ImageDraw

SIZE = 256
BG = (20, 20, 24, 255)
WHITE = (245, 245, 250, 255)
GREEN = (48, 209, 88, 255)


def make_icon() -> Image.Image:
    img = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)

    # Скруглённый тёмный фон
    d.rounded_rectangle((8, 8, SIZE - 8, SIZE - 8), radius=56, fill=BG)

    # Силуэт мыши
    d.rounded_rectangle((82, 46, 174, 214), radius=46, outline=WHITE, width=9)
    d.line((84, 108, 172, 108), fill=WHITE, width=9)

    # Колесо
    d.rounded_rectangle((119, 68, 137, 96), radius=7, fill=GREEN)

    # Небольшая батарейка внизу
    d.rounded_rectangle((103, 170, 153, 192), radius=5, outline=WHITE, width=4)
    d.rectangle((122, 166, 134, 170), fill=WHITE)
    d.rectangle((108, 175, 130, 187), fill=GREEN)

    return img


def main() -> None:
    out = Path(__file__).with_name("icon.ico")
    img = make_icon()
    img.save(out, sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
    img.save(Path(__file__).with_name("icon_preview.png"))
    print(f"written {out}")


if __name__ == "__main__":
    main()
