#!/usr/bin/env python3
"""Regenerates the raster brand assets under site/static/ from the SVG masters.

Created or overwritten in site/static/:
  logo-16.png, logo-16@2x.png, logo-24.png, logo-24@2x.png,
  logo-32.png, logo-32@2x.png,
  mark-ink@2x.png, mark-white@2x.png, mark-cobalt@2x.png,
  lockup-ink@2x.png, lockup-white@2x.png,
  favicon.ico

Sources (read, never written):
  site/static/logo-mark.svg           22x32 master, three column and bridge paths
  site/static/logo-16.svg             11x16 pixel-aligned small master
  site/static/fonts/manrope-800.woff2 the committed subset used for the wordmark

The Open Graph PNGs under site/static/og/ are regenerated separately by
tools/brand/render-og.sh, which renders site/static/og/template.html with
headless Chromium.

Geometry is rasterized with exact per-pixel polygon coverage, so the 1x and 2x
masters keep the integer edges of the vector originals and scaled or offset
renders get mathematically correct anti-aliasing. The lockup wordmark is drawn
from the committed Manrope 800 subset with FreeType (Pillow) at a 4x
supersampling factor. favicon.ico is assembled as four BMP entries (16, 24, 32,
48) so the file stays readable by legacy icon consumers.

Requirements: python3 with Pillow (FreeType with WOFF2 support, as shipped in
the standard Pillow wheels). fontTools is only needed when Pillow cannot read
WOFF2 directly.

Usage:
  python3 tools/brand/render_assets.py [--root /path/to/repo]

The script is idempotent: it always regenerates every asset from the current
masters and fails loudly if an output does not pass its own checks.
"""

from __future__ import annotations

import argparse
import math
import re
import struct
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

INK = (0x18, 0x2C, 0x31)
PAPER = (0xF5, 0xF6, 0xF1)
WHITE = (0xFF, 0xFF, 0xFF)
COBALT = (0x28, 0x49, 0xD8)

TEXT_SUPERSAMPLE = 4

LOCKUP_CSS_WIDTH = 176
LOCKUP_CSS_HEIGHT = 56
LOCKUP_TEXT_X = 30
LOCKUP_TEXT_SIZE = 44
LOCKUP_TEXT_BASELINE = 32
LOCKUP_TEXT_TRACKING_EM = -0.02

Polygon = list[tuple[float, float]]
Shape = tuple[str, Polygon, tuple[int, int, int] | None]


class ParseError(Exception):
    """Raised when an SVG master does not match the expected structure."""


def parse_svg(path: Path) -> list[Shape]:
    """Extracts the drawable shapes from one of the brand masters.

    Returns a list of shapes. A shape is ("poly", [(x, y), ...], fill); fill is
    None when the element relies on a group or root fill.
    """
    source = path.read_text(encoding="utf-8")
    view = re.search(r'viewBox="([^"]+)"', source)
    if view is None:
        raise ParseError(f"{path}: missing viewBox")
    if len([float(value) for value in re.findall(r"-?\d+(?:\.\d+)?", view.group(1))]) != 4:
        raise ParseError(f"{path}: unexpected viewBox {view.group(1)!r}")

    shapes: list[Shape] = []
    stack: list[tuple[float, float]] = []
    offset_x = offset_y = 0.0
    token = re.compile(r"<(/?)(svg|g|rect|path)\b([^>]*?)(/?)>", re.S)
    for match in token.finditer(source):
        closing, name, attrs, _self_closing = match.groups()
        if closing:
            if name == "g":
                stack.pop()
                offset_x, offset_y = stack[-1] if stack else (0.0, 0.0)
            continue
        if name == "g":
            translate = re.search(r'transform="translate\(([-\d.]+)\s+([-\d.]+)\)"', attrs)
            dx = float(translate.group(1)) if translate else 0.0
            dy = float(translate.group(2)) if translate else 0.0
            stack.append((offset_x + dx, offset_y + dy))
            offset_x, offset_y = stack[-1]
            continue
        fill = parse_fill(attrs)
        if name == "rect":
            x, y, width, height = shape_rect(attrs)
            polygon = [
                (x + offset_x, y + offset_y),
                (x + offset_x + width, y + offset_y),
                (x + offset_x + width, y + offset_y + height),
                (x + offset_x, y + offset_y + height),
            ]
            shapes.append(("poly", polygon, fill))
        elif name == "path":
            data = re.search(r'd="([^"]+)"', attrs)
            if data is None:
                raise ParseError(f"{path}: path without d attribute")
            shapes.append(("poly", path_points(data.group(1), offset_x, offset_y), fill))
    if not shapes:
        raise ParseError(f"{path}: no drawable shapes found")
    return shapes


def parse_fill(attrs: str) -> tuple[int, int, int] | None:
    found = re.search(r'fill="([^"]+)"', attrs)
    if found is None or found.group(1) == "currentColor":
        return None
    value = found.group(1).lstrip("#")
    if len(value) != 6:
        raise ParseError(f"unsupported fill {found.group(1)!r}")
    return (int(value[0:2], 16), int(value[2:4], 16), int(value[4:6], 16))


def shape_rect(attrs: str) -> tuple[float, float, float, float]:
    def attr(name: str, default: float | None = None) -> float:
        found = re.search(rf'{name}="([-\d.]+)"', attrs)
        if found is None:
            if default is None:
                raise ParseError(f"rect missing {name}")
            return default
        return float(found.group(1))

    return attr("x", 0.0), attr("y", 0.0), attr("width"), attr("height")


def path_points(data: str, dx: float, dy: float) -> Polygon:
    """Parses the absolute M/L/Z path form used by every brand master."""
    points: Polygon = []
    command = re.compile(r"([MLZ])([^MLZ]*)")
    for match in command.finditer(data):
        verb = match.group(1)
        args = [float(value) for value in re.findall(r"-?\d+(?:\.\d+)?", match.group(2))]
        if verb == "M":
            points.append((args[0] + dx, args[1] + dy))
        elif verb == "L":
            if len(args) % 2 != 0:
                raise ParseError(f"odd L argument count in {data!r}")
            for index in range(0, len(args), 2):
                points.append((args[index] + dx, args[index + 1] + dy))
    if not points:
        raise ParseError(f"path without drawable points: {data!r}")
    return points


def clipped_area(polygon: Polygon, x0: float, y0: float, x1: float, y1: float) -> float:
    """Exact area of a convex polygon clipped to the rectangle (Sutherland-Hodgman)."""
    edges = [
        (lambda p: p[0] - x0, False),
        (lambda p: x1 - p[0], False),
        (lambda p: p[1] - y0, False),
        (lambda p: y1 - p[1], False),
    ]
    points = polygon
    for value, _ in edges:
        if not points:
            return 0.0
        output: Polygon = []
        previous = points[-1]
        previous_value = value(previous)
        for current in points:
            current_value = value(current)
            if current_value >= 0.0:
                if previous_value < 0.0:
                    ratio = previous_value / (previous_value - current_value)
                    output.append((previous[0] + ratio * (current[0] - previous[0]),
                                   previous[1] + ratio * (current[1] - previous[1])))
                output.append(current)
            elif previous_value >= 0.0:
                ratio = previous_value / (previous_value - current_value)
                output.append((previous[0] + ratio * (current[0] - previous[0]),
                               previous[1] + ratio * (current[1] - previous[1])))
            previous, previous_value = current, current_value
        points = output
    if len(points) < 3:
        return 0.0
    area = 0.0
    for index in range(len(points)):
        ax, ay = points[index]
        bx, by = points[(index + 1) % len(points)]
        area += ax * by - bx * ay
    return abs(area) / 2.0


def rasterize_shapes(
    shapes: list[Shape],
    out_size: tuple[int, int],
    unit_scale: float,
    default_color: tuple[int, int, int],
    offset: tuple[float, float] = (0.0, 0.0),
) -> Image.Image:
    """Renders the shapes at exact per-pixel coverage on a transparent canvas."""
    width, height = out_size
    colors = {fill or default_color for _kind, _geometry, fill in shapes}
    if len(colors) != 1:
        raise ParseError("rasterize_shapes expects a single fill color per master")
    color = colors.pop()
    coverage = [0.0] * (width * height)
    for _kind, geometry, _fill in shapes:
        points = [(x * unit_scale + offset[0], y * unit_scale + offset[1]) for x, y in geometry]
        min_x = max(0, math.floor(min(x for x, _ in points)))
        max_x = min(width - 1, math.floor(max(x for x, _ in points)))
        min_y = max(0, math.floor(min(y for _, y in points)))
        max_y = min(height - 1, math.floor(max(y for _, y in points)))
        for py in range(min_y, max_y + 1):
            for px in range(min_x, max_x + 1):
                area = clipped_area(points, float(px), float(py), float(px + 1), float(py + 1))
                if area > 0.0:
                    index = py * width + px
                    coverage[index] = area + coverage[index] * (1.0 - area)
    image = Image.new("RGBA", (width, height), (0, 0, 0, 0))
    pixels = image.load()
    for index, value in enumerate(coverage):
        if value > 0.0:
            pixels[index % width, index // width] = (*color, round(255 * min(1.0, value)))
    return image


def icon_image(master: list[Shape], size: int, unit_scale: float, mark_width: float) -> Image.Image:
    """Centers a mark render on a paper square for one favicon.ico entry."""
    if abs((size - mark_width) / 2.0 - round((size - mark_width) / 2.0)) < 1e-9:
        offset = (size - mark_width) / 2.0
    else:
        offset = (size - mark_width + 1) // 2.0
    back = Image.new("RGBA", (size, size), (*PAPER, 255))
    mark = rasterize_shapes(master, (size, size), unit_scale, INK, offset=(offset, 0.0))
    return Image.alpha_composite(back, mark)


def load_wordmark_font(woff2_path: Path, size: int) -> ImageFont.FreeTypeFont:
    try:
        return ImageFont.truetype(woff2_path, size)
    except OSError:
        from fontTools.ttLib import TTFont  # only needed when FreeType lacks WOFF2

        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            ttf_path = Path(tmp) / "manrope-800.ttf"
            font = TTFont(woff2_path)
            font.flavor = None
            font.save(ttf_path)
            return ImageFont.truetype(ttf_path, size)


def draw_wordmark(
    canvas: Image.Image,
    text: str,
    font: ImageFont.FreeTypeFont,
    start_x: float,
    baseline_y: float,
    tracking_px: float,
    color: tuple[int, int, int],
) -> None:
    """Draws the text glyph by glyph so pair kerning and tracking both apply."""
    draw = ImageDraw.Draw(canvas)
    x = start_x
    for index, char in enumerate(text):
        draw.text((x, baseline_y), char, font=font, fill=(*color, 255), anchor="ls")
        next_char = text[index + 1] if index + 1 < len(text) else ""
        kern = 0.0
        if next_char:
            kern = font.getlength(char + next_char) - font.getlength(char) - font.getlength(next_char)
        x += font.getlength(char) + kern + tracking_px


def render_lockup(root: Path, mark: list[Shape], color: tuple[int, int, int]) -> Image.Image:
    """Renders site/static/logo.svg (176x56) at 2x: 352x112."""
    width, height = LOCKUP_CSS_WIDTH * 2, LOCKUP_CSS_HEIGHT * 2
    lockup = rasterize_shapes(mark, (width, height), 2.0, color)
    sample = TEXT_SUPERSAMPLE
    text_layer = Image.new("RGBA", (width * sample, height * sample), (0, 0, 0, 0))
    font = load_wordmark_font(
        root / "site" / "static" / "fonts" / "manrope-800.woff2",
        LOCKUP_TEXT_SIZE * 2 * sample,
    )
    draw_wordmark(
        text_layer,
        "Apex",
        font,
        start_x=LOCKUP_TEXT_X * 2 * sample,
        baseline_y=LOCKUP_TEXT_BASELINE * 2 * sample,
        tracking_px=LOCKUP_TEXT_TRACKING_EM * LOCKUP_TEXT_SIZE * 2 * sample,
        color=color,
    )
    lockup.alpha_composite(text_layer.resize((width, height), Image.Resampling.BOX))
    return lockup


def encode_bmp_entry(image: Image.Image) -> bytes:
    """Encodes one 32-bit BMP entry for the ICO directory."""
    width, height = image.size
    pixels = image.load()
    xor = bytearray()
    for y in range(height - 1, -1, -1):
        for x in range(width):
            red, green, blue, alpha = pixels[x, y]
            xor += bytes((blue, green, red, alpha))
    mask_row_bytes = ((width + 31) // 32) * 4
    mask = bytearray()
    for y in range(height - 1, -1, -1):
        row = bytearray(mask_row_bytes)
        for x in range(width):
            if pixels[x, y][3] < 128:
                row[x // 8] |= 0x80 >> (x % 8)
        mask += row
    header = struct.pack("<IiiHHIIiiII", 40, width, height * 2, 1, 32, 0, len(xor) + len(mask), 0, 0, 0, 0)
    return bytes(header) + bytes(xor) + bytes(mask)


def write_ico(path: Path, images: list[Image.Image]) -> None:
    entries = bytearray()
    payload = bytearray()
    offset = 6 + 16 * len(images)
    for image in images:
        blob = encode_bmp_entry(image)
        width, height = image.size
        entries += struct.pack("<BBBBHHII", width % 256, height % 256, 0, 0, 1, 32, len(blob), offset)
        payload += blob
        offset += len(blob)
    path.write_bytes(struct.pack("<HHH", 0, 1, len(images)) + bytes(entries) + bytes(payload))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    args = parser.parse_args()
    static: Path = args.root / "site" / "static"

    mark = parse_svg(static / "logo-mark.svg")
    small = parse_svg(static / "logo-16.svg")
    if len(mark) != 3 or len(small) != 3:
        raise SystemExit("expected three paths in both mark masters")

    # logo-32@2x.png and mark-ink@2x.png are the same raster; render it once.
    mark_ink_2x = rasterize_shapes(mark, (44, 64), 2.0, INK)
    outputs = [
        ("logo-16.png", rasterize_shapes(small, (11, 16), 1.0, INK)),
        ("logo-16@2x.png", rasterize_shapes(small, (22, 32), 2.0, INK)),
        ("logo-24.png", rasterize_shapes(mark, (17, 24), 0.75, INK)),
        ("logo-24@2x.png", rasterize_shapes(mark, (33, 48), 1.5, INK)),
        ("logo-32.png", rasterize_shapes(mark, (22, 32), 1.0, INK)),
        ("logo-32@2x.png", mark_ink_2x),
        ("mark-ink@2x.png", mark_ink_2x),
        ("mark-white@2x.png", rasterize_shapes(mark, (44, 64), 2.0, WHITE)),
        ("mark-cobalt@2x.png", rasterize_shapes(mark, (44, 64), 2.0, COBALT)),
        ("lockup-ink@2x.png", render_lockup(args.root, mark, INK)),
        ("lockup-white@2x.png", render_lockup(args.root, mark, WHITE)),
    ]
    for name, image in outputs:
        image.save(static / name, format="PNG")
        print(f"wrote site/static/{name} {image.size[0]}x{image.size[1]}")

    icons = [
        icon_image(small, 16, 1.0, 11.0),
        icon_image(mark, 24, 0.75, 16.5),
        icon_image(mark, 32, 1.0, 22.0),
        icon_image(mark, 48, 1.5, 33.0),
    ]
    write_ico(static / "favicon.ico", icons)

    with Image.open(static / "favicon.ico") as check:
        sizes = sorted(check.ico.sizes())
    expected = [(16, 16), (24, 24), (32, 32), (48, 48)]
    if sizes != expected:
        raise SystemExit(f"favicon.ico reports {sizes}, expected {expected}")
    print("wrote site/static/favicon.ico 16/24/32/48")


if __name__ == "__main__":
    main()
