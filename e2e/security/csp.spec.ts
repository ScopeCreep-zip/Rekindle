import { test, expect } from "@playwright/test";
import { readEnforcedCsp, NO_CSP_REASON } from "../fixtures/csp";

// CSP enforcement tests.
//
// Two different questions live here, and only one is answerable from a
// browser test:
//
//   1. "Is the declared policy strict?" — answered in Rust by
//      src-tauri/tests/csp_policy.rs, which reads the same
//      tauri.conf.json the app ships with and fails CI if a directive is
//      weakened. That runs on every `cargo test`, no browser needed.
//
//   2. "Does the WebView actually enforce it?" — that is what this file
//      is for, and it requires a context that serves the CSP. Tauri
//      injects it through its custom protocol, so under the Vite dev
//      server (which is what Playwright drives) there is no CSP at all.
//
// So these tests skip themselves when no CSP is present rather than
// asserting nothing. They previously used `if (meta) { ... }` guards,
// which meant "no CSP" silently satisfied every assertion — and because
// `locator.getAttribute()` auto-waits for a missing element, each one
// also burned the full 30s test timeout before getting there.

test.describe("CSP — enforced policy", () => {
  test("response carries a Content-Security-Policy meta or header", async ({
    page,
  }) => {
    await page.goto("/login");
    const csp = await readEnforcedCsp(page);
    test.skip(csp === null, NO_CSP_REASON);
    expect(csp, "CSP must be declared via meta or header").toBeTruthy();
  });

  test("CSP forbids 'unsafe-eval'", async ({ page }) => {
    await page.goto("/login");
    const csp = await readEnforcedCsp(page);
    test.skip(csp === null, NO_CSP_REASON);
    expect(csp).not.toContain("'unsafe-eval'");
  });

  test("CSP forbids 'unsafe-inline' for script-src", async ({ page }) => {
    await page.goto("/login");
    const csp = await readEnforcedCsp(page);
    test.skip(csp === null, NO_CSP_REASON);
    // Allowed in style-src (Tailwind/SolidJS need it); forbidden in
    // script-src.
    const scriptSrc = (csp!.match(/script-src[^;]*/) ?? [""])[0];
    expect(scriptSrc).not.toContain("'unsafe-inline'");
    expect(scriptSrc).not.toContain("'unsafe-eval'");
  });

  test("CSP includes object-src 'none'", async ({ page }) => {
    await page.goto("/login");
    const csp = await readEnforcedCsp(page);
    test.skip(csp === null, NO_CSP_REASON);
    expect(csp).toMatch(/object-src\s+'none'/);
  });

  test("CSP includes frame-ancestors 'none'", async ({ page }) => {
    await page.goto("/login");
    const csp = await readEnforcedCsp(page);
    test.skip(csp === null, NO_CSP_REASON);
    expect(csp).toMatch(/frame-ancestors\s+'none'/);
  });

  test("CSP forbids mixed content (no remote http: in connect-src)", async ({
    page,
  }) => {
    await page.goto("/login");
    const csp = await readEnforcedCsp(page);
    test.skip(csp === null, NO_CSP_REASON);
    const connectSrc = (csp!.match(/connect-src[^;]*/) ?? [""])[0];
    // The known Tauri internal hostnames are allowed; arbitrary http is
    // not.
    const allowedHosts = ["ipc:", "ipc.localhost", "asset.localhost"];
    const tokens = connectSrc
      .split(/\s+/)
      .filter((t) => t.startsWith("http:") || t.startsWith("https:"));
    for (const token of tokens) {
      const ok = allowedHosts.some((h) => token.includes(h));
      expect(ok, `connect-src contains non-allowlisted host: ${token}`).toBe(
        true,
      );
    }
  });
});
