# The shell's icon set

`src/components/common/glyph_art.ts` holds the SVG outlines of the icons the Community
window draws (plan SK17-7a, `SkinGlyph`), written by `scripts/skin_glyphs.py`.

## Source

- Font: **Symbols Nerd Font Mono**, `Version 001.000;Nerd Fonts 3.3.0` (the font's own name table), the file
  `src/assets/fonts/SymbolsNerdFontMono-Regular.woff2` the app ships
- Glyphs: **Material Design Icons** by Pictogrammers (https://pictogrammers.com), as
  carried in Nerd Fonts (https://www.nerdfonts.com)
- Icons taken: 39, at the codepoints `src/icons.ts` names
- Licence: Apache License 2.0, full text in `LICENSE-Apache-2.0.txt` (the canonical
  text, sha256 cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30)

The outlines are copied unchanged, in the font's em square. No font is loaded to draw them.
