#!/usr/bin/env python3
"""draw the launcher icon of each size, icons/<size>/sailtree.png, and the picture shown behind
the cover, qml/icons/cover.png

the launcher icon is the same drawing as icons/sailtree.svg, which is the reference: keep both in
sync. drawn with Pillow because the build environments have no svg renderer.
    python3 scripts/make-icons.py
"""
import os
from PIL import Image, ImageDraw

SIZES = [86, 108, 128, 172]
# the cover picture is made as the ones of the system's apps (graphic-cover-*.png of the theme):
# the size of a cover, white only, the picture large, cut by the edges and mostly transparent
COVER = (352, 562)
# drawn bigger then reduced, for smooth edges
OVERSAMPLING = 8
BOTTOM_LEFT, TOP_RIGHT = (0x2e, 0x2e, 0x2e), (0x8a, 0x8a, 0x8a)
WHITE = (255, 255, 255)


def tree(size, scale, left, top, opacities):
    """the tree of items in white on a transparent image of `size` pixels. `scale` is the size of
    an icon unit (the icon is 86 units wide), `left` and `top` place the origin of the icon.
    `opacities` are those of the branches, of the top item and of the two others"""
    def s(*values):
        # x and y alternate
        return [value * scale + (top if index % 2 else left) for index, value in enumerate(values)]

    picture = Image.new("RGBA", size, (0, 0, 0, 0))

    # each part on its own layer for its transparency
    def layer(opacity, paint):
        nonlocal picture
        overlay = Image.new("RGBA", size, (0, 0, 0, 0))
        paint(ImageDraw.Draw(overlay), WHITE + (round(255 * opacity),))
        picture = Image.alpha_composite(picture, overlay)

    def branches(pen, color):
        width = round(3 * scale)
        for line in [(27, 33, 27, 60), (27, 60, 38, 60), (27, 44, 38, 44)]:
            pen.line(s(*line), fill=color, width=width)
        # round ends and corner
        for x, y in [(27, 33), (27, 60), (38, 60), (38, 44)]:
            pen.ellipse(s(x - 1.5, y - 1.5, x + 1.5, y + 1.5), fill=color)

    def item(x, y):
        return lambda pen, color: pen.rounded_rectangle(s(x, y, x + 26, y + 9), radius=4.5 * scale, fill=color)

    layer(opacities[0], branches)
    layer(opacities[1], item(20, 22))
    for y in (39.5, 55.5):
        layer(opacities[2], item(40, y))
    return picture


def icon(size):
    scale = size * OVERSAMPLING / 86
    side = size * OVERSAMPLING

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
    ImageDraw.Draw(mask).ellipse([value * scale for value in (0.3, 0.3, 85.7, 85.7)], fill=255)

    result = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    result.paste(gradient, (0, 0), mask)
    result = Image.alpha_composite(result, tree((side, side), scale, 0, 0, (0.75, 1.0, 0.9)))
    return result.resize((size, size), Image.LANCZOS)


def cover():
    width, height = COVER
    size = (width * OVERSAMPLING, height * OVERSAMPLING)
    # the picture is wider than the cover: its items leave by the right edge
    scale = 9.6 * OVERSAMPLING
    left = (30 - 20 * 9.6) * OVERSAMPLING
    top = (height - 42.5 * 9.6) / 2 * OVERSAMPLING - 22 * scale
    return tree(size, scale, left, top, (0.07, 0.16, 0.11)).resize(COVER, Image.LANCZOS)


def main():
    root = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
    for size in SIZES:
        folder = os.path.join(root, "icons", f"{size}x{size}")
        os.makedirs(folder, exist_ok=True)
        icon(size).save(os.path.join(folder, "sailtree.png"))
        print(f"icons/{size}x{size}/sailtree.png")
    cover().save(os.path.join(root, "qml", "icons", "cover.png"))
    print("qml/icons/cover.png")


if __name__ == "__main__":
    main()
