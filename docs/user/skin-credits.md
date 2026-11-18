# Skin credits and removal

Rekindle ships a few skins built in. Three are Xfire's own skins (Symbiosis,
Classic v3, Combat). The others are fan skins made by the Xfire community:
Onyx, SpaceCowboy, StealthX, Total Chaos, xSLIMFIRE, xBIOSHOCK, Guild Wars UI,
Alloy, Simple X and XGlow.

## Credits

Each built-in fan skin is credited to the authors its own files name (the
skin's `Info.xml` `<Author>` entries, or the comments in its `version.ini`).
Rekindle ships no fan skin whose files name nobody, and none whose author
restricted it ("All rights reserved", "do not change this skin"). To see who
made a skin, open **Settings → Appearance** and press the **i** button
beside it. The credits panel shows the authors and contributors, the site
the skin names, and its licence text, or that it states none. Each skin's
`NOTICE` file in `crates/rekindle-skin/builtin/` records where the skin came
from and everything the import changed. Colours lightened or darkened for
legibility are listed there.

A skin you import yourself shows its credits the same way, labelled "As
stated by the skin (not verified)". Rekindle records what the skin says
about itself and does not check it.

## Asking for removal

If you made one of the built-in skins and don't want Rekindle to ship it,
open an issue on the project's tracker,
<https://github.com/ScopeCreep-zip/Rekindle/issues>. Name the skin and say
how you're connected to it, for example the site or handle the skin
credits. Your skin is removed in the next release, and nobody will ask you
why.

The tracker is public. If you'd rather not ask in public, use the private
reporting path in
[`../security/vulnerability-disclosure.md`](../security/vulnerability-disclosure.md)
§3 and say the report is a skin removal request.
