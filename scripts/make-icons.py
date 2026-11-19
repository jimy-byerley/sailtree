#!/usr/bin/env python3
"""draw the launcher icon of each size, icons/<size>/sailtree.png

same drawing as icons/sailtree.svg, which is the reference: keep both in sync. drawn with
Pillow because the build environments have no svg renderer.
    python3 scripts/make-icons.py
"""
import os
from PIL import Image, ImageDraw

SIZES = [86, 108, 128, 172]
# drawn bigger then reduced, for smooth edges
OVERSAMPLING = 8
BOTTOM_LEFT, TOP_RIGHT = (0x2e, 0x2e, 0x2e), (0x8a, 0x8a, 0x8a)
WHITE = (255, 255, 255)


def draw(size):
    scale = size * OVERSAMPLING / 86
    side = size * OVERSAMPLING

    def s(*values):
        return [value * scale for value in values]

    # background: gradient from the bottom left to the top right
    gradient = Image.new("RGB", (side, side))
    pixels = gradient.load()
    for y in range(side):
        for x in range(side):
            # position along the diagonal from (8, 78) to (78, 8), in icon units
            t = ((x / scale - 8) + (78 - y / scale)) / 140
            t = min(1, max(0, t))
            pixels[x, y] = tuple(round(a + (b - a) * t) for a, b in zip(BOTTOM_LEFT, TOP_RIGHT))

    # base shape: the fully rounded one, a circle
    mask = Image.new("L", (side, side), 0)
    ImageDraw.Draw(mask).ellipse(s(0.3, 0.3, 85.7, 85.7), fill=255)

    icon = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    icon.paste(gradient, (0, 0), mask)

    # the tree of items, on its own layer for the transparency of its parts
    def layer(opacity, paint):
        overlay = Image.new("RGBA", (side, side), (0, 0, 0, 0))
        paint(ImageDraw.Draw(overlay), WHITE + (round(255 * opacity),))
        return Image.alpha_composite(icon, overlay)

    def branches(pen, color):
        width = round(3 * scale)
        for line in [(27, 33, 27, 60), (27, 60, 38, 60), (27, 44, 38, 44)]:
            pen.line(s(*line), fill=color, width=width)
        # round ends and corner
        for x, y in [(27, 33), (27, 60), (38, 60), (38, 44)]:
            pen.ellipse(s(x - 1.5, y - 1.5, x + 1.5, y + 1.5), fill=color)

    icon = layer(0.75, branches)
    icon = layer(1.0, lambda pen, color: pen.rounded_rectangle(s(20, 22, 46, 31), radius=4.5 * scale, fill=color))
    for top in (39.5, 55.5):
        icon = layer(0.9, lambda pen, color: pen.rounded_rectangle(s(40, top, 66, top + 9), radius=4.5 * scale, fill=color))
    return icon.resize((size, size), Image.LANCZOS)


def main():
    root = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "icons")
    for size in SIZES:
        folder = os.path.join(root, f"{size}x{size}")
        os.makedirs(folder, exist_ok=True)
        draw(size).save(os.path.join(folder, "sailtree.png"))
        print(f"icons/{size}x{size}/sailtree.png")


if __name__ == "__main__":
    main()
