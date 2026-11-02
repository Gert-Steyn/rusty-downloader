"""Draw the NSIS installer images from the app icon.

    python tools/dev/installer_images.py

Needs Pillow. Writes src-tauri/windows/sidebar.bmp (164x314, the welcome and
finish pages) and src-tauri/windows/header.bmp (150x57, the top right of the
other pages), as 24-bit BMPs since NSIS can't show transparency.
"""

from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[2]
ICON = ROOT / "src-tauri" / "icons" / "icon.png"
OUT = ROOT / "src-tauri" / "windows"
FONT = Path("C:/Windows/Fonts/seguisb.ttf")

# The icon's own blues, top and bottom.
TOP = (21, 101, 192)
BOTTOM = (11, 61, 145)


def gradient(size):
    width, height = size
    image = Image.new("RGB", size)
    draw = ImageDraw.Draw(image)
    for y in range(height):
        t = y / max(height - 1, 1)
        color = tuple(round(a + (b - a) * t) for a, b in zip(TOP, BOTTOM))
        draw.line([(0, y), (width, y)], fill=color)
    return image


def icon(size):
    return Image.open(ICON).convert("RGBA").resize((size, size), Image.LANCZOS)


def sidebar():
    image = gradient((164, 314))
    mark = icon(88)
    # A soft white tile behind the icon, so it stands off the same blue.
    tile = Image.new("RGBA", (104, 104), (0, 0, 0, 0))
    ImageDraw.Draw(tile).rounded_rectangle(
        [0, 0, 103, 103], radius=22, fill=(255, 255, 255, 46)
    )
    image.paste(tile, (30, 70), tile)
    image.paste(mark, (38, 78), mark)
    draw = ImageDraw.Draw(image)
    font = ImageFont.truetype(str(FONT), 17)
    for i, line in enumerate(["Rusty", "Downloader"]):
        width = draw.textlength(line, font=font)
        draw.text(((164 - width) / 2, 196 + i * 23), line, font=font, fill="white")
    return image


def header():
    image = Image.new("RGB", (150, 57), "white")
    mark = icon(41)
    image.paste(mark, (150 - 41 - 8, 8), mark)
    return image


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    sidebar().save(OUT / "sidebar.bmp")
    header().save(OUT / "header.bmp")
    print(f"wrote {OUT / 'sidebar.bmp'} and {OUT / 'header.bmp'}")


if __name__ == "__main__":
    main()
