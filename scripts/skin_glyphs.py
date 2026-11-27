#!/usr/bin/env python3
"""Write src/components/common/glyph_art.ts: the shell's own icon set (plan
SK17-7a), the outlines of the glyphs a skinned window used through Symbols
Nerd Font Mono, as SVG path data, and src/assets/icons/NOTICE.md.

A skinned window draws these as inline SVG in the role's colour, never as a
font glyph. Each glyph keeps the font's em square (units per em, flipped at
the ascent), so at 1em it sits where and at the size the font glyph did.

Source: src/assets/fonts/SymbolsNerdFontMono-Regular.woff2 (its name and
version are read from the font and recorded in the NOTICE). The glyph map
references src/icons.ts constants where the app also uses the font glyph.
Symbols Nerd Font bundles several icon sets under different licences. Each
glyph is mapped by its font name and its codepoint range, and a
glyph of a set with no recorded licence is refused. The NOTICE lists the
sets actually used, each with its licence text in src/assets/icons/.

Usage (fontTools with brotli): python3 scripts/skin_glyphs.py
"""

import re
import sys
from pathlib import Path

from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont

ROOT = Path(__file__).resolve().parent.parent
FONT = ROOT / "src/assets/fonts/SymbolsNerdFontMono-Regular.woff2"
ICONS = ROOT / "src/icons.ts"
OUT = ROOT / "src/components/common/glyph_art.ts"
NOTICE = ROOT / "src/assets/icons/NOTICE.md"

# The icon sets whose licence is recorded: the glyph-name prefix the font
# gives the set, its Nerd Fonts codepoint range, and its licence file in
# src/assets/icons/. A glyph from any other set is refused.
SETS = [
    {
        "prefix": "md-",
        "range": (0xF0001, 0xF1AF0),
        "name": "Material Design Icons",
        "by": "Pictogrammers (https://pictogrammers.com)",
        "licence": "Apache License 2.0",
        "file": "LICENSE-Apache-2.0.txt",
        "sha256": "cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30",
    },
]

# Glyph name -> the src/icons.ts constant where the app also shows the font
# glyph, or a codepoint (with its Nerd Fonts name) for an icon only a skinned
# window shows.
# The Community window's roles use the Host's names (community_icon,
# src/styles/generated/community_icon.ts); the rest are the shell's own.
GLYPHS = {
    "voice.server": "ICON_VOLUME_HIGH",
    "mic.on": 0xF036C,  # nf-md-microphone
    "mic.off": 0xF036D,  # nf-md-microphone_off
    "speaker.on": 0xF02CB,  # nf-md-headphones
    "speaker.muted": 0xF0581,
    "speaker.off": 0xF07CE,  # nf-md-headphones_off
    "fold.open": 0xF0140,  # nf-md-chevron_down
    "fold.closed": 0xF0142,  # nf-md-chevron_right
    "room": "ICON_COMMUNITIES",
    "chat": 0xF0423,  # nf-md-pound
    "account-remove": "ICON_ACCOUNT_REMOVE",
    "archive": 0xF003C,  # nf-md-archive
    "arrow-down": 0xF0045,  # nf-md-arrow_down
    "arrow-up": 0xF005D,  # nf-md-arrow_up
    "ban": 0xF029B,  # nf-md-gavel
    "bell": "ICON_BELL",
    "check": "ICON_CHECK",
    "check-all": 0xF012D,  # nf-md-check_all
    "close": 0xF0156,  # nf-md-close
    "close-circle": 0xF0159,  # nf-md-close_circle
    "copy": 0xF018F,  # nf-md-content_copy
    "delete": 0xF01B4,  # nf-md-delete
    "dots": 0xF01D8,  # nf-md-dots_horizontal
    "download": 0xF01DA,  # nf-md-download
    "emoticon": 0xF01F2,  # nf-md-emoticon_outline
    "eye": 0xF0208,  # nf-md-eye
    "file": 0xF0214,  # nf-md-file
    "file-image": 0xF021F,  # nf-md-file_image
    "folder": 0xF024B,  # nf-md-folder
    "folder-open": 0xF0770,  # nf-md-folder_open
    "forward": 0xF0496,  # nf-md-share
    "gamepad": "ICON_GAMEPAD",
    "hangup": "ICON_HANGUP",
    "key": "ICON_KEY",
    "lock": 0xF033E,
    "logout": "ICON_LOGOUT",
    "megaphone": 0xF00E6,  # nf-md-bullhorn
    "open-in-new": 0xF03CC,  # nf-md-open_in_new
    "paperclip": 0xF03E2,  # nf-md-paperclip
    "pause": 0xF03E4,
    "pencil": 0xF03EB,  # nf-md-pencil
    "perms": 0xF099D,  # nf-md-shield_lock
    "phone": "ICON_PHONE",
    "pin": 0xF0403,  # nf-md-pin
    "pip": 0xF0E57,  # nf-md-picture_in_picture_bottom_right
    "play": 0xF040A,
    "plus": "ICON_PLUS",
    "plus-box": 0xF0416,  # nf-md-plus_box
    "refresh": 0xF0450,  # nf-md-refresh
    "reply": 0xF045A,  # nf-md-reply
    "save": 0xF0193,  # nf-md-content_save
    "screen-share": 0xF1483,  # nf-md-monitor_share
    "search": 0xF0349,  # nf-md-magnify
    "server": 0xF048B,  # nf-md-server
    "settings": 0xF0493,  # nf-md-cog
    "shield": "ICON_SHIELD",
    "thread": 0xF0367,  # nf-md-message_reply
    "timeout": 0xF0954,  # nf-md-clock
    "timer-sand": 0xF051F,  # nf-md-timer_sand
    "video": "ICON_VIDEO",
    "video-off": 0xF0568,  # nf-md-video_off
}


def codepoints() -> dict:
    out = {}
    for m in re.finditer(r'export const (ICON_[A-Z0-9_]+) = "\\u\{([0-9A-Fa-f]+)\}"', ICONS.read_text()):
        out[m.group(1)] = int(m.group(2), 16)
    return out


def set_of(glyph_name: str, cp: int):
    """The recorded set `glyph_name` at `cp` belongs to, by both its name in the font and its range."""
    for s in SETS:
        lo, hi = s["range"]
        if glyph_name.startswith(s["prefix"]) and lo <= cp <= hi:
            return s
    return None


def main() -> int:
    font = TTFont(str(FONT))
    cmap = font.getBestCmap()
    glyphs = font.getGlyphSet()
    em = font["head"].unitsPerEm
    ascent = font["hhea"].ascent
    cps = codepoints()
    lines = []
    used = {}
    for name, source in GLYPHS.items():
        cp = cps.get(source) if isinstance(source, str) else source
        if cp is None:
            print(f"{source} is not in src/icons.ts", file=sys.stderr)
            return 1
        glyph = cmap.get(cp)
        if glyph is None:
            print(f"{name} U+{cp:X} is not in the font", file=sys.stderr)
            return 1
        s = set_of(glyph, cp)
        if s is None:
            print(f"{name} U+{cp:X} ({glyph}) is from an icon set with no recorded licence; refusing", file=sys.stderr)
            return 1
        used.setdefault(s["name"], (s, []))[1].append(name)
        pen = SVGPathPen(glyphs)
        # Font units, y up from the baseline -> SVG, y down from the em top.
        glyphs[glyph].draw(TransformPen(pen, (1, 0, 0, -1, 0, ascent)))
        advance = font["hmtx"][glyph][0]
        key = name if re.fullmatch(r"[A-Za-z_$][A-Za-z0-9_$]*", name) else f'"{name}"'
        left = (advance - em) // 2
        path = pen.getCommands()
        inline = f'  {key}: {{ x: {left}, d: "{path}" }},'
        # Match the checked-in Biome width when laying out each outline.
        if len(inline) <= 100:
            lines.append(inline)
        else:
            lines.append(f'  {key}: {{\n    x: {left},\n    d: "{path}",\n  }},')
    family = font["name"].getDebugName(1)
    version = font["name"].getDebugName(5)
    sets_line = "; ".join(f"{s['name']} by {s['by'].split(' (')[0]}, {s['licence']}" for s, _ in used.values())
    OUT.write_text(
        "// Generated by scripts/skin_glyphs.py from src/assets/fonts/SymbolsNerdFontMono-Regular.woff2\n"
        f"// ({family}, {version}) with the codepoints named in the generator. Do not edit: regenerate.\n"
        f"// {sets_line}: src/assets/icons/NOTICE.md.\n\n"
        "/** One glyph: its path in the font's em square, and the square's left edge. */\n"
        "export interface GlyphArt {\n  x: number;\n  d: string;\n}\n\n"
        f"/** The em square's side, in font units. */\nexport const GLYPH_EM = {em};\n\n"
        "/** The shell's icon set, by name (plan SK17-7a). */\n"
        "export const GLYPH_ART = {\n" + "\n".join(lines) + "\n} as const satisfies Record<string, GlyphArt>;\n\n"
        "export type GlyphName = keyof typeof GLYPH_ART;\n"
    )
    print(f"wrote {OUT.relative_to(ROOT)}: {len(lines)} glyphs")
    sections = []
    for s, names in used.values():
        lo, hi = s["range"]
        sections.append(
            f"### {s['name']}\n\n"
            f"- By {s['by']}, as carried in Nerd Fonts (https://www.nerdfonts.com): glyph names `{s['prefix']}…`,\n"
            f"  codepoints U+{lo:X}–U+{hi:X}\n"
            f"- Licence: {s['licence']}, full text in `{s['file']}` (sha256 {s['sha256']})\n"
            f"- Icons taken ({len(names)}): {', '.join(f'`{n}`' for n in names)}\n"
        )
    NOTICE.write_text(
        "# The shell's icon set\n\n"
        "`src/components/common/glyph_art.ts` holds the SVG outlines of the icons a skinned window\n"
        "draws (plan SK17-7a, `SkinGlyph`), written by `scripts/skin_glyphs.py`.\n\n"
        "## Source\n\n"
        f"- Font: **{family}**, `{version}` (the font's own name table), the file\n"
        "  `src/assets/fonts/SymbolsNerdFontMono-Regular.woff2` the app ships\n"
        f"- {len(lines)} icons at the codepoints in the generator and `src/icons.ts`. Symbols Nerd Font bundles several\n"
        "  icon sets under different licences; each icon is mapped to its set by its glyph name and its\n"
        "  codepoint range, and an icon of a set with no recorded licence is refused. The sets used:\n\n"
        + "\n".join(sections)
        + "\nThe outlines are copied unchanged, in the font's em square. No font is loaded to draw them.\n"
    )
    print(f"wrote {NOTICE.relative_to(ROOT)}: sets used: {', '.join(used)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
