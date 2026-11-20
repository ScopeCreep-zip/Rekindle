// Plan E7.14 S4: E7.12's geometry checks in the real app's own windows, on
// the engine users run (WKWebView, WebKitGTK), over real data: the login
// window on first run, then an identity with a 64-character unbroken name
// (MAX_DISPLAY_NAME_LEN, the old stress name) in the buddy list and
// Settings; then the volume one user makes alone, made for real at E7.12's
// old stress scale (12 communities, 30 channels, 40 messages) with names at
// their limits (key_format::MAX_NAME_LEN, 100) and an unbroken URL longer
// than any window. Each window at its minimum, default and large size.

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, byText, shown, switchTo, waitForHandle } from "../../../walk/helpers.ts";
import { LOGIN as LOGIN_LABEL, SETTINGS as SETTINGS_LABEL, expectOnlySelectedTab, openSettings } from "../../../walk/nav.ts";
import { createCommunity, createIdentity, createTextChannel, newUser, openCommunity, postMessage, selectChannel, type User } from "../../journeys.ts";
import {
  BUDDY_LIST,
  BUDDY_LIST_COMMUNITIES,
  COMMUNITY,
  LOGIN,
  SETTINGS_APPEARANCE,
  SETTINGS_APPEARANCE_CREDITS,
  SETTINGS_PRIVACY,
  SETTINGS_PROFILE,
  SKIN_EDITOR,
  checkWindow,
  sizeWindow,
  sizes,
} from "../../windows.ts";

/** A name of exactly `n` characters with no spaces: the worst case for wrapping. */
function longName(prefix: string, n = 64): string {
  return (prefix + "W".repeat(n)).slice(0, n);
}

/** The fewest whole lines the skin editor's licence field shows (plan E7.7.5). */
const MIN_LICENCE_LINES = 6;

/** The most a community or channel name may hold (`key_format::MAX_NAME_LEN`). */
const MAX_NAME = 100;

/** An unbroken URL longer than any window is wide (E7.12's `LONG_URL`). */
const LONG_URL = `https://example.com/${"averyveryverylongpathsegment".repeat(12)}?q=${"x".repeat(80)}`;

describe("layout of the real windows", () => {
  const me: User = { ...newUser("layout"), displayName: longName("Layout_") };

  it("login window on first run", async () => {
    await switchTo(browser, LOGIN_LABEL);
    await expect(await checkWindow(browser, LOGIN, { withData: false })).toEqual({});
  });

  it("buddy list of a 64-character name", async () => {
    await createIdentity(browser, me);
    await expect(await checkWindow(browser, BUDDY_LIST, { withData: false })).toEqual({});
  });

  for (const [tab, w] of [["Profile", SETTINGS_PROFILE], ["Privacy", SETTINGS_PRIVACY], ["Appearance", SETTINGS_APPEARANCE]] as const) {
    it(`settings, ${tab} tab`, async () => {
      await openSettings(browser, tab);
      await expectOnlySelectedTab(browser, tab);
      if (tab === "Appearance") await byText(browser, ".skin-row", "");
      await switchTo(browser, SETTINGS_LABEL);
      await expect(await checkWindow(browser, w, { withData: true })).toEqual({});
      await browser.closeWindow();
    });
  }

  it("settings, Appearance with Symbiosis's credits open", async () => {
    await openSettings(browser, "Appearance");
    await (await shown(browser, '[aria-label="Credits for Symbiosis"]')).click();
    await shown(browser, ".skin-credits");
    await expect(await checkWindow(browser, SETTINGS_APPEARANCE_CREDITS, { withData: true })).toEqual({});
  });

  it("skin editor window", async () => {
    // From the Settings window the previous test left open.
    await switchTo(browser, SETTINGS_LABEL);
    const before = await browser.getWindowHandles();
    await (await byText(browser, "button.form-btn-secondary", "Edit a skin")).click();
    await browser.switchToWindow(await waitForHandle(browser, (h) => h === "skin-editor" && !before.includes(h), NET_WAIT_MS));
    await shown(browser, ".skin-pair", NET_WAIT_MS);
    await expect(await checkWindow(browser, SKIN_EDITOR, { withData: true })).toEqual({});
    // Plan E7.7.5 (moved from e2e/layout/skin-editor-licence.spec.ts): the
    // licence field takes the whole row and shows at least six whole lines
    // (the Linux walk saw it clip mid-line at the default size).
    for (const [sizeName, size] of sizes(SKIN_EDITOR).filter(([n]) => n !== "large")) {
      await sizeWindow(browser, size);
      const m = await browser.execute(() => {
        const el = document.querySelector<HTMLTextAreaElement>(".skin-editor-licence-field textarea");
        if (!el) return null;
        const s = getComputedStyle(el);
        const meta = el.closest(".skin-editor-meta")!.getBoundingClientRect();
        return {
          content: el.clientHeight - parseFloat(s.paddingTop) - parseFloat(s.paddingBottom),
          line: parseFloat(s.lineHeight),
          width: el.getBoundingClientRect().width,
          row: meta.width,
        };
      });
      if (!m) throw new Error(`${sizeName}: no licence field`);
      const lines = m.content / m.line;
      expect(Number.isFinite(m.line)).toBe(true);
      expect(lines).toBeGreaterThanOrEqual(MIN_LICENCE_LINES);
      expect(Math.abs(lines - Math.round(lines)) * m.line).toBeLessThan(1);
      expect(m.width).toBeGreaterThan(m.row - 1);
    }
    await browser.closeWindow();
    await switchTo(browser, SETTINGS_LABEL);
    await browser.closeWindow();
  });

  const community = longName("Community_", MAX_NAME);
  const channel = longName("channel_", MAX_NAME);

  it("buddy list's Communities tab with 12 communities", async () => {
    await createCommunity(browser, me, community);
    for (let i = 1; i < 12; i++) await createCommunity(browser, me, `layout community ${i}`);
    await expect(await checkWindow(browser, BUDDY_LIST_COMMUNITIES, { withData: true })).toEqual({});
    // Twelve real community creates (each a DHT record set) outlast mocha's 300 s default.
  }).timeout(900_000);

  it("community window with 30 channels and 40 messages", async () => {
    await openCommunity(browser, community);
    await createTextChannel(browser, me, channel);
    for (let i = 2; i < 30; i++) await createTextChannel(browser, me, `channel-${String(i).padStart(2, "0")}`);
    await selectChannel(browser, channel);
    await postMessage(browser, me, LONG_URL);
    await postMessage(browser, me, longName("unbroken_", 300));
    for (let i = 3; i <= 40; i++) await postMessage(browser, me, `message ${i} of the layout run`);
    await expect(await checkWindow(browser, COMMUNITY, { withData: true })).toEqual({});
    // 29 channel creates and 40 posts, each a real Host write.
  }).timeout(900_000);
});
