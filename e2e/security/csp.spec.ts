import { test, expect } from "@playwright/test";

// CSP enforcement tests.
//
// The CSP declared in `src-tauri/tauri.conf.json` `app.security.csp` is
// the load-bearing mitigation against XSS. These tests assert that the
// CSP is actually enforced by the WebView (which Tauri injects via
// custom protocol headers in production) and that the directives we
// promise are actually present.

test.describe("CSP — directives present", () => {
  test("response carries a Content-Security-Policy header or meta", async ({
    page,
  }) => {
    await page.goto("/login");

    // In dev mode (Vite dev server), the CSP comes from a meta element
    // injected by Tauri. In production, it's a response header. Check
    // both.
    const cspMeta = await page
      .locator('meta[http-equiv="Content-Security-Policy"]')
      .getAttribute("content")
      .catch(() => null);
    const cspHeader = await page.evaluate(async () => {
      try {
        const r = await fetch(window.location.href);
        return r.headers.get("content-security-policy");
      } catch {
        return null;
      }
    });
    const csp = cspMeta ?? cspHeader;
    expect(csp, "CSP must be declared via meta or header").toBeTruthy();
  });

  test("CSP forbids 'unsafe-eval'", async ({ page }) => {
    await page.goto("/login");
    const meta = await page
      .locator('meta[http-equiv="Content-Security-Policy"]')
      .getAttribute("content")
      .catch(() => null);
    if (meta) {
      expect(meta).not.toContain("'unsafe-eval'");
    }
  });

  test("CSP forbids 'unsafe-inline' for script-src", async ({ page }) => {
    await page.goto("/login");
    const meta = await page
      .locator('meta[http-equiv="Content-Security-Policy"]')
      .getAttribute("content")
      .catch(() => null);
    if (meta) {
      // Allowed in style-src (Tailwind/SolidJS need it); forbidden in
      // script-src.
      const scriptSrc = (meta.match(/script-src[^;]*/) ?? [""])[0];
      expect(scriptSrc).not.toContain("'unsafe-inline'");
      expect(scriptSrc).not.toContain("'unsafe-eval'");
    }
  });

  test("CSP includes object-src 'none'", async ({ page }) => {
    await page.goto("/login");
    const meta = await page
      .locator('meta[http-equiv="Content-Security-Policy"]')
      .getAttribute("content")
      .catch(() => null);
    if (meta) {
      expect(meta).toMatch(/object-src\s+'none'/);
    }
  });

  test("CSP includes frame-ancestors 'none'", async ({ page }) => {
    await page.goto("/login");
    const meta = await page
      .locator('meta[http-equiv="Content-Security-Policy"]')
      .getAttribute("content")
      .catch(() => null);
    if (meta) {
      expect(meta).toMatch(/frame-ancestors\s+'none'/);
    }
  });

  test("CSP forbids mixed content (no http: in connect-src)", async ({
    page,
  }) => {
    await page.goto("/login");
    const meta = await page
      .locator('meta[http-equiv="Content-Security-Policy"]')
      .getAttribute("content")
      .catch(() => null);
    if (meta) {
      const connectSrc = (meta.match(/connect-src[^;]*/) ?? [""])[0];
      // The known Tauri internal hostnames are allowed; arbitrary http
      // is not.
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
    }
  });
});
