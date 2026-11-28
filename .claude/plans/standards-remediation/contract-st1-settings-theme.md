# ST1 token contract: Settings follows the theme (2026-11-26)

Owning steps: plan E7.7.3 item 1 (the importer's colour map) and E7.7.7 (windows follow the pack).
Research: `research-settings-theme-2026-11-26.md` §2.2, §3.2, §3.3. Backend half: branch `wt/st1`
(prefix `ST1:`). This file is the contract for the frontend half: the CSS changes in
`src/styles/theme/*.css`, `src/styles/scrollbar.css` and the Settings components.

## Rules the backend guarantees

- Every re-imported built-in pack's token file sets **every** role below (test
  `crates/rekindle-skin/tests/dialog_roles.rs::every_builtin_sets_every_dialog_role`). The value is the pack's
  own `Themes.xml` / INI colour when it names one, or the mean of its own art for the two art grounds
  (`titlebar.*`, `menubar.bg`; `crates/rekindle-skin/src/import/dialog_roles.rs`), else the derivation in the
  "Derivation" column over the colours it does define. No registry default reaches a built-in for these roles.
- **One derivation table** (lead ruling ST1-4, `ruling-st1-walk-2026-11-26.md`): the derivations are the
  registry rows (`crates/rekindle-skin/src/registry/dialog.rs`; `text.disabled` is a base row; the title grounds
  are dialog rows, and the base walk holds `titlebar.text` to their defaults as before the roles existed). The importer writes what `registry::resolve` serves (`registry::dialog::fill`), and `resolve` serves
  the same rows at load to a package that lacks a role: xfire-classic (no source tree) gets every role at load
  (test `xfire_classic_is_served_every_dialog_role_from_its_own_colours`).
- **Walk order** (rulings ST1-1, ST1-2): the base pairs are walked first with every role held out, so a pack's
  own colours settle exactly as before the roles existed; then each role the pack does not name is derived once
  from the settled base; then the dialog pairs are walked, moving **only a dialog role**. A pack colour never
  moves for a dialog constraint.
- Each role carries its contrast constraint in the registry, so the validator and the import walk hold every
  pair; any walk change is listed in the pack's `NOTICE`. **A text role is held only against the ground the
  pack itself draws it on** (ruling ST1-3); a pack-named text role on a ground the pack never drew it on is a
  contract error, never a walk target.
- A skin of another format that omits a role follows the registry derivation (shown in brackets; `text.base`
  follows `text.primary` when a skin sets only that, so the bracket values equal the earlier `text.primary`
  ones).
- The served stylesheet emits each property as written below (`src/styles/generated/tokens.css` declares them
  all, with `@property` registrations).

## The table

| Role | CSS custom property | Xfire colour (XML / INI) or derivation | Contrast gate | Settings element(s) (§3.2) |
| --- | --- | --- | --- | --- |
| `menubar.bg` | `--color-xfire-menubar-bg` | The alpha-weighted mean colour of the pack's menu-bar art (`MenuBackground` / `MenuBar`, the `buddylist.menubar` control; numbers only, no art), or its named colour; a pack with neither: `bg.dark` [`bg.dark`] | ground of `menubar.text` (4.5:1) | The tab rail's ground behind the tab labels (the `.form-tab` row) |
| `menubar.text` | `--color-xfire-menubar-text` | `MenuTextCG` / `MenuTextCG`; else `text.base` [`text.base`] | 4.5:1 on `menubar.bg` only (Xfire draws `MenuTextCG` only on its menu bar; ruling ST1-3) | Tab labels (`.form-tab`), replacing `--color-xfire-text-dim`, **drawn on `menubar.bg`, never on `bg.dark`** |
| `input.bg` | `--color-xfire-input-bg` | `TextFieldBGCG`; else `bg.panel` (the chat view ground the INI edit box uses) [`bg.panel`] | ground | Text inputs and closed selects (`.form-input`, `.form-select`), checkbox/radio box ground, replacing `--color-xfire-bg-input` there |
| `input.text` | `--color-xfire-input-text` | `TextFieldTextCG`; else `text.base` [`text.base`] | 4.5:1 on `input.bg`, `input.selection.bg-inactive` | Input and closed-select text, `caret-color` |
| `input.selection.bg` | `--color-xfire-input-selection-bg` | `TextFieldTextHighlightCG`; else `accent.fill` [`accent.fill`] | ground of `selection.text` | `::selection` background while the window is focused |
| `input.selection.bg-inactive` | `--color-xfire-input-selection-bg-inactive` | `TextFieldTextHighlightInactiveCG`; else `accent.fill` 40 % over `input.bg` [same] | ground of `input.text` | `::selection` background while the window is unfocused (text stays `input.text`) |
| `selection.text` | `--color-xfire-selection-text` | `ListColorSelectedTextCG` / `BuddyViewSelectedText`, Combat `Label` (no longer the fixed `accent.fg`); else `text.base` [`text.base`] | 4.5:1 on `accent.fill`, `input.selection.bg` | `::selection` text (focused); text and check mark on the selected fill: active tab label (if drawn solid), primary button text, chosen skin row text, the custom listbox's selected row, checkbox/radio mark |
| (selected fill) | `--color-xfire-accent` | `ListColorSelectedCG` / `BuddyViewSelected`, `Select` (unchanged) | n/a | Active tab underline or fill, primary button ground, chosen skin row (solid), listbox selected row, checked checkbox/radio fill |
| `text.disabled` | `--color-xfire-text-disabled` | `DisabledTextCG` (Symbiosis) / `DisabledText`; else the base row's derivation, `text.primary` 86 % over `bg.primary` [same] (one table, ruling ST1-4) | 4.5:1 on every text ground | Disabled controls and their labels (`:disabled`), replacing `opacity: 0.4/0.5` |
| `progress.bg` | `--color-xfire-progress-bg` | `ProgBarBGCG`; else `bg.panel` [`bg.panel`] | ground | Range slider track (`NotificationsTab.tsx:144`) |
| `progress.fill` | `--color-xfire-progress-fill` | `ProgBarFillCG` / `ProgressFill`; else `accent.fill` [`accent.fill`] | 3:1 on `progress.bg` | Range slider fill and thumb |
| `progress.outline` | `--color-xfire-progress-outline` | `ProgBarOutlineCG`; else `text.base` 50 % over `progress.bg` [same] | 3:1 on `bg.dark` | Slider track outline; **control borders** (inputs, closed selects, checkbox/radio boxes), replacing the fixed-default `--color-xfire-control-border`: it is the theme's only control-outline colour and is gated at WCAG 1.4.11's 3:1 on the window ground |
| `progress.text` | `--color-xfire-progress-text` | `ProgBarTextCG`; else `text.base` [`text.base`] | 4.5:1 on `progress.bg` (Xfire draws `ProgBarTextCG` on its bar's well; ruling ST1-3 replaces the earlier `bg.panel` pairing) | **Not used by Settings**, which draws no text on the bar; a value label beside the slider is a hint (`--color-xfire-text-status`, below) |
| `scrollbar.track` | `--color-xfire-scrollbar-track` | `ScrollbarBGCG` / `ScrollbarColorBG`; else `bg.primary` [`bg.primary`] | ground | Scrollbar track (`scrollbar.css:8`), replacing `bg-dark` |
| `scrollbar.thumb` | `--color-xfire-scrollbar-thumb` | Xfire has none (its thumb is art); `text.base` 50 % over `scrollbar.track` [same] | 3:1 on `scrollbar.track` | Scrollbar thumb (`scrollbar.css:12-22`), replacing `--color-xfire-offline` 40/60/80 %. Hover/active: `color-mix` of the thumb toward `--color-xfire-menubar-text` in CSS |
| `tooltip.bg` | `--color-xfire-tooltip-bg` | `ToolTipBGCG` / `TooltipCG`; else `bg.secondary` (the info view) [`bg.secondary`] | ground | Tooltips (when added) |
| `tooltip.text` | `--color-xfire-tooltip-text` | `ToolTipTextCG` / `TooltipTextCG`; else `text.base` [`text.base`] | 4.5:1 on `tooltip.bg` | Tooltip text |
| `titlebar.start` | `--color-xfire-titlebar-start` | The alpha-weighted mean colour of the **left end** piece of the pack's buddy-list title strip (numbers only, no art); a pack with no strip: `accent.fill` 50 % over `bg.primary` [same] | ground of `titlebar.text` (4.5:1) | Settings title-bar gradient start (`.xfire-titlebar` with SB1's region var unset) |
| `titlebar.end` | `--color-xfire-titlebar-end` | The mean colour of the strip's **right end** piece; no strip: `accent.fill` 25 % over `bg.primary` [same] | ground of `titlebar.text` (4.5:1) | Settings title-bar gradient end |

## Elements that use existing tokens (CSS only, no new role)

| Settings element | Token | Source |
| --- | --- | --- |
| Section titles (`.settings-section-title`) | `--color-xfire-text-secondary` | `HeaderTextCG` / `HeaderText` |
| Hints, field labels and a slider's value label beside it (`.settings-hint`, `.form-field-label`) | `--color-xfire-text-status` | `StatusTextCG` / `StatusCG` |
| Secondary and skin-row buttons | ground `--color-xfire-bg-secondary`, text `--color-xfire-text`, border `--color-xfire-progress-outline` | `InfoViewBGCG`, `ListFriendTextCG`, `ProgBarOutlineCG` |
| Row hover (`.settings-option:hover`) | **derived in CSS**: `color-mix(in srgb, var(--color-xfire-accent) 12%, transparent)` over the ground, replacing the `overlay` 5 % tint (the contrast model measures `token N% over ground` tints) | `ListColorSelectedCG` |
| Title buttons' hover | **derived in CSS**: `color-mix(in srgb, var(--color-xfire-titlebar-text) 15%, transparent)` | `ChatTitleCG` |
| Links, focus ring | `--color-xfire-link`, `--color-xfire-focus-outline` (unchanged) | `LinkTextCG` |
| Danger, error | registry danger tokens (unchanged; Xfire has no role) | n/a |

The frontend half reruns `cargo xtask gen-contrast-pairs` after the CSS change, so the model measures every
new pair (selection, slider, thumb, borders) against the grounds `src/styles/regions.toml` declares.
