# The shell's icon set

`src/components/common/glyph_art.ts` holds the SVG outlines of the icons a skinned window
draws (plan SK17-7a, `SkinGlyph`), written by `scripts/skin_glyphs.py`.

## Source

- Font: **Symbols Nerd Font Mono**, `Version 001.000;Nerd Fonts 3.3.0` (the font's own name table), the file
  `src/assets/fonts/SymbolsNerdFontMono-Regular.woff2` the app ships
- 61 icons at the codepoints in the generator and `src/icons.ts`. Symbols Nerd Font bundles several
  icon sets under different licences; each icon is mapped to its set by its glyph name and its
  codepoint range, and an icon of a set with no recorded licence is refused. The sets used:

### Material Design Icons

- By Pictogrammers (https://pictogrammers.com), as carried in Nerd Fonts (https://www.nerdfonts.com): glyph names `md-…`,
  codepoints U+F0001–U+F1AF0
- Licence: Apache License 2.0, full text in `LICENSE-Apache-2.0.txt` (sha256 cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30)
- Icons taken (61): `voice.server`, `mic.on`, `mic.off`, `speaker.on`, `speaker.muted`, `speaker.off`, `fold.open`, `fold.closed`, `room`, `chat`, `account-remove`, `archive`, `arrow-down`, `arrow-up`, `ban`, `bell`, `check`, `check-all`, `close`, `close-circle`, `copy`, `delete`, `dots`, `download`, `emoticon`, `eye`, `file`, `file-image`, `folder`, `folder-open`, `forward`, `gamepad`, `hangup`, `key`, `lock`, `logout`, `megaphone`, `open-in-new`, `paperclip`, `pause`, `pencil`, `perms`, `phone`, `pin`, `pip`, `play`, `plus`, `plus-box`, `refresh`, `reply`, `save`, `screen-share`, `search`, `server`, `settings`, `shield`, `thread`, `timeout`, `timer-sand`, `video`, `video-off`

The outlines are copied unchanged, in the font's em square. No font is loaded to draw them.
