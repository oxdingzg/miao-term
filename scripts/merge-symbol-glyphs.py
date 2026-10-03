#!/usr/bin/env python3
"""Merge monochrome symbol glyphs into the bundled JetBrains Mono.

cosmic-text's macOS fallback for a named family is fixed:
JetBrains Mono -> .SF NS -> Menlo -> Apple Color Emoji -> ... -> bundled fonts.
Text-presentation emoji such as U+23FA (Claude Code's message marker) exist in
none of the fonts before Apple Color Emoji, so they render as colour bitmaps
that ignore the terminal foreground. Adding the glyphs to the primary font
keeps them monochrome and tinted like any other text.

Sources are Noto fonts (SIL OFL 1.1, no Reserved Font Name):
  https://github.com/notofonts/notofonts.github.io (fonts/<Family>/unhinted/ttf)

Usage: merge-symbol-glyphs.py <noto-font-dir> [font.ttf]
The directory must contain NotoSansSymbols2-Regular.ttf,
NotoSansSymbols-Regular.ttf and NotoSans-Regular.ttf. The font is rewritten in
place; running the script again is a no-op.
"""

import os
import sys

from fontTools.pens.recordingPen import DecomposingRecordingPen
from fontTools.pens.transformPen import TransformPen
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import TTFont

# Text-default emoji (Emoji=Yes, Emoji_Presentation=No) that reach Apple Color
# Emoji before any monochrome font on macOS, limited to symbols a monochrome
# macOS font also draws (so native terminals show them as text). Pictographs
# such as U+1F5A5 stay colour emoji, as they do in Terminal.app and iTerm2.
SYMBOLS = {
    "NotoSans-Regular.ttf": [0x2139],
    "NotoSansSymbols-Regular.ttf": [0x26A7],
    "NotoSansSymbols2-Regular.ttf": [
        0x23ED, 0x23EE, 0x23EF, 0x23F1, 0x23F2, 0x23F8, 0x23F9, 0x23FA,
        0x2B05, 0x2B06, 0x2B07,
    ],
}

# Fit the symbol inside one cell, like the font's own U+25CF.
MAX_WIDTH = 580


def main():
    src_dir = sys.argv[1]
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    path = sys.argv[2] if len(sys.argv) > 2 else os.path.join(
        root, "assets", "fonts", "JetBrainsMono.ttf")
    font = TTFont(path)
    # Decompile every table touched below before the glyph order grows.
    for tag in ("cmap", "glyf", "hmtx", "gvar", "HVAR"):
        font[tag]
    advance = font["hmtx"]["A"][0]
    cmap = font.getBestCmap()
    order = list(font.getGlyphOrder())
    glyf = font["glyf"]
    hvar = font["HVAR"].table
    added = []

    for file, codepoints in SYMBOLS.items():
        src = TTFont(os.path.join(src_dir, file))
        src_cmap = src.getBestCmap()
        glyphs = src.getGlyphSet()
        for cp in codepoints:
            if cp in cmap:
                continue
            name = "uni%04X" % cp
            src_name = src_cmap[cp]
            src_glyph = src["glyf"][src_name]
            src_glyph.recalcBounds(src["glyf"])
            x0, y0, x1, y1 = (src_glyph.xMin, src_glyph.yMin,
                              src_glyph.xMax, src_glyph.yMax)
            scale = min(1.0, MAX_WIDTH / (x1 - x0))
            cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
            # Scale about the glyph centre, then centre it in the cell.
            transform = (scale, 0, 0, scale,
                         advance / 2 - cx * scale, cy - cy * scale)
            outline = DecomposingRecordingPen(glyphs)
            glyphs[src_name].draw(outline)
            pen = TTGlyphPen(None)
            outline.replay(TransformPen(pen, transform))
            glyph = pen.glyph()
            glyph.recalcBounds(None)

            order.append(name)
            glyf.glyphs[name] = glyph
            font["hmtx"][name] = (advance, glyph.xMin)
            font["gvar"].variations[name] = []
            # Same (non-varying) advance as every other glyph.
            hvar.AdvWidthMap.mapping[name] = hvar.AdvWidthMap.mapping["A"]
            for table in font["cmap"].tables:
                if table.isUnicode():
                    table.cmap[cp] = name
            added.append(cp)

    if not added:
        print("nothing to merge")
        return
    font.setGlyphOrder(order)
    glyf.glyphOrder = order
    font.save(path)
    print("merged", " ".join("U+%04X" % cp for cp in added), "into", path)


if __name__ == "__main__":
    main()
