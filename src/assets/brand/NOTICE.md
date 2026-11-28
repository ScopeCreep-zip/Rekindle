# Rekindle brand mark

- `rekindle-mark.svg`: the flame, traced by hand from the desktop icon
  (`logo/rekindle.png`, `src-tauri/icons/`); gradient stops sampled from that icon.
- `rekindle-mark-small.svg`: the same flame with two layers, for boxes of 16 px or less.
- `rekindle-wordmark.svg`: the flame and the word "Rekindle".

## Font in the wordmark

- Font: **Outfit**, SemiBold (weight 600 instance of the variable font), version 1.100
- Copyright 2021 The Outfit Project Authors (https://github.com/Outfitio/Outfit-Fonts)
- Source: https://github.com/google/fonts/tree/main/ofl/outfit (`Outfit[wght].ttf`)
- Licence: SIL Open Font License 1.1, full text in `OFL-Outfit.txt`

The glyphs are outlined to SVG paths (fontTools, HarfBuzz shaping with kerning). No font file
is shipped, and the font is not loaded at runtime.
