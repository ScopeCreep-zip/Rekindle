# Security E2E Suite

Web-app security tests for the Tauri webview surface. The Tauri shell
runs the SolidJS frontend in a real browser engine (WebKit / WebView2 /
WebKitGTK), so XSS, CSP-bypass, DOM-injection, deep-link injection,
and prototype-pollution risks all apply. These tests assert that the
defences are actually in place at runtime.

## Suites

| File | What it tests |
|------|---------------|
| [`xss.spec.ts`](xss.spec.ts) | Inject XSS payloads into every peer-content render path; assert no script execution. |
| [`csp.spec.ts`](csp.spec.ts) | Assert the CSP declared in `src-tauri/tauri.conf.json` is actually enforced (script-src, frame-ancestors, eval, mixed content). |
| [`deep-link.spec.ts`](deep-link.spec.ts) | Hostile `rekindle://` payload corpus — empty, garbage, XSS-in-URL, huge-payload, null-bytes, RTL override. |

## Running

```sh
# Local
pnpm exec playwright test --config playwright.security.config.ts

# CI
# .github/workflows/lint.yml `security-e2e` job (added when this suite is wired in)
```

## Adding a new test

Whenever a new peer-content render path lands (markdown bodies, link
previews, custom-emoji names, profile bios, etc.), add a test in
[`xss.spec.ts`](xss.spec.ts) that mocks the IPC response with the
shared `XSS_PAYLOADS` corpus and asserts no execution.

The skipped tests at the bottom of `xss.spec.ts` are placeholder
tripwires — un-skip them when the corresponding feature ships.

## Mocking IPC

Security tests run in **mock-IPC mode** (`VITE_PLAYWRIGHT=true`) so
they don't depend on a Veilid node. The mock fixtures live at
[`../fixtures/mocks.ts`](../fixtures/mocks.ts). When mocking a new
IPC command, prefer extending the existing `LOGIN_SUCCESS_HANDLER`
shape over inventing a new pattern.

## Reading reports

When a test fails, Playwright produces an HTML report. The most
useful artefacts:

- **`window.__rekindleXssTriggered`** — boolean set by an XSS payload
  if a script executed.
- **`window.__rekindleEvalCalled`** — string set by an `eval`
  invocation; populated by the canary in `installXssCanary`.
- **`window.__rekindleErrors`** — array of caught `error` and
  `unhandledrejection` events.

If `__rekindleXssTriggered === true` is observed, file a `P0` per
[`../../docs/security/incident-response.md`](../../docs/security/incident-response.md).

## Out of scope

- Tests that need a live Veilid node — those go in the regular `e2e/`
  project and run under `playwright.config.ts`.
- Network-level attacks (MITM, DNS hijack) — those are covered by the
  encryption layer, not the rendering layer.
- WebView CVEs — tracked separately by `.github/workflows/webview-cve-check.yml`.
