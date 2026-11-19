import { expect, test, type Page } from "@playwright/test";

/**
 * How long a window's first paint may take. On a cold dev server under load,
 * Vite transforms a lazy window's modules on its first request, which can
 * exceed the 5 s expect default; this is launch-sized, not a layout budget.
 */
export const FIRST_PAINT_MS = 20_000;

/**
 * Wait for the window's frame to exist; it asserts nothing about layout.
 *
 * The wait is added to the test's own time: a 20 s first paint inside the
 * 30 s test default left the rest of a loaded test (networkidle, axe, its
 * checks) too little, so it timed out "waiting for the window frame".
 */
export function firstPaint(page: Page) {
  const info = test.info();
  info.setTimeout(info.timeout + FIRST_PAINT_MS);
  return expect(page.locator(".app-frame").first()).toBeVisible({ timeout: FIRST_PAINT_MS });
}
