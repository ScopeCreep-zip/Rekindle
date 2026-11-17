import { test, expect } from "@playwright/test";
import { preloadMocks } from "./fixtures/mocks";
import { PEER, friends, safetyNumber, stressHandler, FULL } from "./fixtures/stress";

// The profile window over a safety-number reply (plan E2.3): a valid one
// draws the number and its QR code; a malformed one (no `qrRows`) costs only
// the QR code, which says so, and never the name, the badge or the digits.

const NAME = friends(1)[0].displayName;

async function openProfile(page: import("@playwright/test").Page, reply: unknown) {
  const base = stressHandler(FULL);
  const handler = `if (cmd === "get_safety_number") return ${JSON.stringify(reply)};\n${base}`;
  await preloadMocks(page, `profile-${PEER.slice(0, 16)}`, handler);
  await page.goto(`/profile?key=${PEER}`);
}

test("a valid safety number draws its digits and QR code", async ({ page }) => {
  await openProfile(page, safetyNumber(PEER));
  await expect(page.locator(".profile-display-name")).toHaveText(NAME);
  await expect(page.locator(".sec-safety-number-digits span")).toHaveCount(12);
  await expect(page.locator(".sec-safety-number-qr-modules")).toHaveAttribute("d", /^M/);
});

test("a malformed safety number costs only the QR code", async ({ page }) => {
  const { qrRows: _dropped, ...malformed } = safetyNumber(PEER);
  await openProfile(page, malformed);
  await expect(page.locator(".profile-display-name")).toHaveText(NAME);
  await expect(page.locator(".sec-safety-number-digits span")).toHaveCount(12);
  await expect(page.locator(".sec-safety-number [role=alert]")).toHaveText(
    "The QR code could not be drawn. Compare the numbers above instead.",
  );
  await expect(page.locator(".sec-safety-number-qr")).toHaveCount(0);
});
