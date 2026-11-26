# tauri-plugin-wdio-webdriver (vendored, patched)

The embedded W3C WebDriver server the whole-app walk drives the debug
desktop app with (plan step E7.13 "Driver"). Built only with the
`webdriver-test` feature of `rekindle-desktop` in a debug build; `cargo xtask
check-webdriver-release` keeps it out of every release build.

## Upstream

- Crate: `tauri-plugin-wdio-webdriver` **1.5.1**, MIT (`LICENSE`, kept as
  shipped), published 2026-10-24.
- Repository: <https://github.com/webdriverio/desktop-mobile>, path
  `packages/tauri-plugin-webdriver`; the crate's `.cargo_vcs_info.json`
  names commit `ba733f664e034976499fd3d43cf940f357f904c4`, `dirty: true`, so
  the crates.io archive, not the git tree, is the source vendored here.
- crates.io checksum (SHA-256 of the `.crate`):
  `5a775b0b59f6f12b38d1f28667311a89cd07f532f540e27d65c6eadfba1156d8`,
  verified against the downloaded archive before vendoring.
- `UPSTREAM-README.md` is the crate's README as shipped, except that its two
  "See Also" links into the upstream monorepo (`../tauri-service/docs/...`)
  point at that commit on GitHub, since the relative paths do not exist here.

## Changes from upstream

1. **Request guard** (`src/server/auth.rs`, wired in `src/server/mod.rs`
   `start`). Upstream binds `127.0.0.1` with no token, `Host` or `Origin`
   check and serves sessionless `POST /wdio/eval`. Every route and the
   fallback now refuse a request whose `Host` is not `127.0.0.1:<port>` or
   that carries any `Origin` (403), or that lacks `Authorization: Bearer
   <token>` with the run's token from `REKINDLE_WDIO_TOKEN`, compared in
   constant time by `rekindle_secrets::bearer_token::BearerToken` (401). The
   server does not start without a valid token. Tests:
   `src/server/auth_tests.rs`.
2. **macOS UI delegate** (`src/platform/macos*`). Upstream replaces wry's
   `WKUIDelegate` with its own, which drops media-capture permission and the
   file open panel. The plugin's delegate now keeps the delegate it
   replaced and forwards those two selectors to it, so camera, microphone
   and file pickers behave as in the shipped app.
3. **Platforms**: macOS and Linux only (the walk's machines); the Windows
   backend and its dependencies are removed and other targets fail to
   compile.
4. **Linux `glib`**: 0.18, the release `webkit2gtk` 2.0 and the workspace
   already use, instead of upstream's separate 0.21 stack.
5. **Workspace lints**: the crate inherits `[lints] workspace = true` with no
   `#[allow]`; long files and functions are split along module seams,
   `std::sync::Mutex` is `parking_lot::Mutex`, and every `unsafe` block
   carries a `// SAFETY:` comment (`docs/contributor/style-guide.md`).
6. **Release guard**: `src/lib.rs` refuses to compile without
   `debug_assertions`.
7. **Linux number conversion** (`src/platform/linux.rs` `js_number`): a
   JavaScript number converts by parsing JavaScriptCore's string form as an
   `i64` (a JSON integer), else as a finite `f64` (a JSON float), else null.
   Upstream compared and cast the `f64` (`n <= i64::MAX as f64`, `n as i64`),
   which the workspace lints refuse (`float_cmp`, `cast_precision_loss`,
   `cast_possible_truncation`) and which let 2^63 through to a saturating
   cast. `js_value_to_json` returns `Value` (it never failed).

Behaviour is otherwise upstream's; upstream's tests are kept.
