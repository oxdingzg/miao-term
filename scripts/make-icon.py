#!/usr/bin/env python3
"""Write a solid-colour 256x256 PNG (no dependencies).

The AppImage tooling refuses a 1x1 placeholder icon, and CI images do not
guarantee ImageMagick, so generate a real icon with the standard library:

    python3 scripts/make-icon.py out.png [rrggbb]
"""

import struct
import sys
import zlib


def chunk(kind: bytes, data: bytes) -> bytes:
    return (
        struct.pack(">I", len(data))
        + kind
        + data
        + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)
    )


def png(size: int, rgb: tuple[int, int, int]) -> bytes:
    header = struct.pack(">IIBBBBB", size, size, 8, 2, 0, 0, 0)  # 8-bit RGB
    row = b"\x00" + bytes(rgb) * size  # filter 0 + pixels
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", header)
        + chunk(b"IDAT", zlib.compress(row * size, 9))
        + chunk(b"IEND", b"")
    )


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    out = sys.argv[1]
    colour = sys.argv[2] if len(sys.argv) > 2 else "2e3440"
    rgb = tuple(int(colour[i : i + 2], 16) for i in (0, 2, 4))
    with open(out, "wb") as fh:
        fh.write(png(256, rgb))  # type: ignore[arg-type]
    print(f"wrote {out} (256x256 #{colour})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
