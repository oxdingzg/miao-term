#!/usr/bin/env python3
"""Write a 256x256 PNG icon with no dependencies.

The AppImage tooling refuses a 1x1 placeholder icon, CI images do not guarantee
ImageMagick, and macOS wants a real icon for the app bundle (and for the About
panel), so draw one with the standard library:

    python3 scripts/make-icon.py out.png [rrggbb]

The design is a rounded dark tile with a warm prompt chevron and a cursor block.
`rrggbb` overrides the tile colour (the AppImage build passes its dark grey).
"""

import struct
import sys
import zlib
SIZE = 256


def chunk(kind: bytes, data: bytes) -> bytes:
    return (
        struct.pack(">I", len(data))
        + kind
        + data
        + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)
    )


def rounded_tile(rgb: tuple[int, int, int]) -> list[list[tuple[int, int, int]]]:
    """A rounded square of `rgb` on a transparent background."""
    px = [[(0, 0, 0)] * SIZE for _ in range(SIZE)]
    r = SIZE * 0.22
    for y in range(SIZE):
        for x in range(SIZE):
            cx = min(max(x, r), SIZE - 1 - r)
            cy = min(max(y, r), SIZE - 1 - r)
            dx, dy = x - cx, y - cy
            if dx * dx + dy * dy <= r * r + r:
                px[y][x] = rgb
    return px


def blend(dst: tuple[int, int, int], src: tuple[int, int, int], a: float):
    return tuple(int(d + (s - d) * a) for d, s in zip(dst, src))


def thick_line(px, x0: float, y0: float, x1: float, y1: float, w: float, colour):
    steps = int(max(abs(x1 - x0), abs(y1 - y0)) * 2) + 1
    for i in range(steps + 1):
        t = i / steps
        cx, cy = x0 + (x1 - x0) * t, y0 + (y1 - y0) * t
        for y in range(max(0, int(cy - w)), min(SIZE, int(cy + w) + 1)):
            for x in range(max(0, int(cx - w)), min(SIZE, int(cx + w) + 1)):
                d = ((x - cx) ** 2 + (y - cy) ** 2) ** 0.5
                if d <= w:
                    px[y][x] = blend(px[y][x], colour, min(1.0, w - d + 0.5))


def rect(px, x0: int, y0: int, x1: int, y1: int, colour):
    for y in range(max(0, y0), min(SIZE, y1)):
        for x in range(max(0, x0), min(SIZE, x1)):
            px[y][x] = colour


def draw(tile: tuple[int, int, int]) -> bytes:
    px = rounded_tile(tile)
    accent = (0x88, 0xC0, 0xD0)  # Nord8, matches the app accent
    warm = (0xEB, 0xCB, 0x8B)  # Nord13
    # ">" prompt chevron.
    thick_line(px, 74, 92, 122, 128, 11, accent)
    thick_line(px, 122, 128, 74, 164, 11, accent)
    # "_" cursor block.
    rect(px, 138, 150, 188, 166, warm)
    raw = b"".join(b"\x00" + bytes(c for p in row for c in p) for row in px)
    header = struct.pack(">IIBBBBB", SIZE, SIZE, 8, 2, 0, 0, 0)  # 8-bit RGB
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(
        b"IDAT", zlib.compress(raw, 9)
    ) + chunk(b"IEND", b"")


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    out = sys.argv[1]
    colour = sys.argv[2] if len(sys.argv) > 2 else "2e3440"
    tile = tuple(int(colour[i : i + 2], 16) for i in (0, 2, 4))
    with open(out, "wb") as fh:
        fh.write(draw(tile))  # type: ignore[arg-type]
    print(f"wrote {out} (256x256 #{colour})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
