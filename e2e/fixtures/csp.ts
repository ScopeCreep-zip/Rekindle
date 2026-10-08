import type { Page } from "@playwright/test";

/**
 * Read the CSP the current context actually enforces, or null when it
 * serves none.
 *
 * Uses `page.evaluate` rather than a locator on purpose: locator queries
 * auto-wait for the element to appear, so a missing CSP meta costs a
 * full test timeout instead of returning null immediately.
 */
export async function readEnforcedCsp(page: Page): Promise<string | null> {
  return page.evaluate(() => {
    const meta = document.querySelector(
      'meta[http-equiv="Content-Security-Policy"]',
    );
    return meta?.getAttribute("content") ?? null;
  });
}

/**
 * Skip reason for tests that need a CSP-enforcing runtime.
 *
 * Tauri injects the policy through its custom protocol, so nothing
 * serves it under the Vite dev server that Playwright drives. The
 * *declared* policy is covered without a browser by
 * `src-tauri/tests/csp_policy.rs`, which fails CI if any directive is
 * weakened; what stays uncovered until these run against a Tauri build
 * is runtime enforcement.
 */
export const NO_CSP_REASON =
  "no CSP served in this context (Vite dev server); Tauri injects it via " +
  "its custom protocol. The declared policy is asserted in " +
  "src-tauri/tests/csp_policy.rs — this test needs a Tauri build to " +
  "verify runtime enforcement.";
