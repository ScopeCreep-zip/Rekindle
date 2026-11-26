# Built-in skin pack sources

The original source trees of the built-in skin packs
(`crates/rekindle-skin/builtin/<pack>.rkskin`), the inputs that
`cargo xtask import-skin` reads. Each tree is stored byte-for-byte as the
importer reads it, under `legacy/skins/<pack>/<folder>/`. The folder keeps the
pack's original name because the INI-era importer
(`rekindle_skin::import::xfire_ini`) names the skin file after it
(`Alloy/Alloy.ini`).

Every tree was verified against the `source tree BLAKE3` in its
`crates/rekindle-skin/builtin/<pack>.NOTICE`: the hash is
`rekindle_skin::import::tree_hash` over every regular file in the folder
(paths `/`-joined, relative to the folder, links not followed), the same walk
as xtask's `read_tree`. Any added, removed or edited file changes it,
stray `.DS_Store` and `.swp` files included: Symbiosis's tree holds five
(`.DS_Store` x4, `Components/.MainWindow.xml.swp`) and they are part of its
hash, so they stay. `.gitignore` re-includes this directory past its `/legacy/`
and `.DS_Store` rules, and `.gitattributes` marks `legacy/**` binary, so git
never rewrites line endings here.

The art and palettes are their authors' work; Rekindle's MIT licence does not
cover them (see each pack's NOTICE for its licence note).

| Pack | Folder | Format | Files | Provenance (from the NOTICE) | Source tree BLAKE3 |
|------|--------|--------|------:|------------------------------|--------------------|
| alloy | `Alloy` | xfire-ini | 216 | LongName Alloy, revision 1; skin file `Alloy.ini` | `66fe43add7be55007861f723062526c0a5cf1e32c328fcd0c16768c8a94c32b0` |
| simple-x | `SimpleX` | xfire-ini | 201 | LongName Simple X, revision 1 | `7f5880e0454c55590e45da6aca0ebe2196de4292ce3476d2a8ccf9ec1b56ebe5` |
| combat | `combat` | xfire-ini | 202 | LongName Combat, revision 2; source `https://web.archive.org/web/20100105213412/http://www.xfire.com/downloads/1032/` | `a580bf8861b8f0a6cb820e7a4c4a112266beba4a104501f8424382fd0f605bdf` |
| xglow | `Xglow` | xfire-ini | 183 | LongName XGlow | `306c159a698c8fa48188ff659b9b5e45571c2bc1fb1f03adaa72d15ea745924a` |
| symbiosis | `Symbiosis` | xfire-xml | 529 | Xfire, Inc., version 1.0; website http://www.xfire.com/ | `b249566d18e88b44c1a8f7e61fb94f0e598836e55b50ac64161ddef7c61b5269` |
| onyx | `Onyx` | xfire-xml | 284 | Gunowar, version 2; website http://www.gunsammocrate.com/ | `788fdbce14c0161fc1bf16abde7d185d5fb09fc2c52f2fd824c963e04bec7f32` |
| xslimfire | `xSLIMFIRE` | xfire-xml | 205 | Grasman, version 1; holds the source correction the NOTICE records (`xSLIMFIREpopups.xml:5`) | `f9aea4aac3d0d7b03cbe22ab67dce228f1c7fffec7e98855a44f07a665db4fbc` |
| stealthx | `StealthX` | xfire-xml | 108 | MrCPU, René Schädlich, version 1; website http://www.xfire.com/ | `9361c4881e29f78713d2e78a40b890ccaf480613a8d5e1987999dbb3d9486603` |
| spacecowboy | `Space_Cowboy` | xfire-xml | 236 | Peanut, Gunowar, version 1 | `1dc982e57376b9f02ad1359471978fa02d4ec45d5a2c25d375b86fa3add663e3` |
| total-chaos | `Total_Chaos` | xfire-xml | 188 | Ryan Dunn, version 0; website http://www.totalchaos.org.uk/ | `eac16e3d4af4bd70a94cbc4ce908d7c3d9658a3d33ce49d1da738478a232a896` |

Still without a stored source: `xfire-classic` (NOTICE hash
`ccb37e23e133ac50693c4d065fa6d7fc246a9f4e3958c4834a8915b448f1bd53`); no copy of
its tree was found.
