# Bundled fonts

These fonts are bundled with `mtty` and embedded into
the binary by `miao-term-render`. They are **not** covered by the project's
Apache-2.0 license; each keeps its own license.

| File | Family | Source | License |
|------|--------|--------|---------|
| `JetBrainsMono.ttf` | JetBrains Mono (Regular) | https://github.com/JetBrains/JetBrainsMono | SIL Open Font License 1.1 — [`LICENSE-OFL-1.1.txt`](LICENSE-OFL-1.1.txt) |
| `JetBrainsMono-Italic.ttf` | JetBrains Mono (Italic) | https://github.com/JetBrains/JetBrainsMono | SIL Open Font License 1.1 — [`LICENSE-OFL-1.1.txt`](LICENSE-OFL-1.1.txt) |
| `SymbolsNerdFontMono-Regular.ttf` | Symbols Nerd Font Mono | https://github.com/ryanoasis/nerd-fonts | MIT License — [`LICENSE-MIT.txt`](LICENSE-MIT.txt) |
| `tabler-icons-subset.ttf` | Tabler Icons (subset) | https://github.com/tabler/tabler-icons | MIT License — [`tabler-LICENSE`](tabler-LICENSE) |

JetBrains Mono is the default monospace terminal font. The Regular file is
modified: `scripts/merge-symbol-glyphs.py` adds monochrome glyphs for
text-presentation emoji (U+23FA and similar) taken from Noto Sans, Noto Sans
Symbols and Noto Sans Symbols 2 (https://github.com/notofonts, SIL Open Font
License 1.1, no Reserved Font Name). Without them macOS font fallback reaches
Apple Color Emoji first and the symbols ignore the terminal foreground colour.
 Symbols Nerd Font Mono is
loaded as a fallback so Powerline / Nerd Font glyphs and icons used by shells
and TUIs render instead of showing tofu.

Tabler Icons (MIT) is subset to the glyphs used by the native host's UI icon
font (`tabler-LICENSE` in this directory) and is the icon font for
`mtty` / `miao-term-ui`.
