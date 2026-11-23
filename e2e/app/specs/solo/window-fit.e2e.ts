// Plan E7.14 for SK2-N4's window fit (round 24): a tree-laid window is held
// to its skin's served frame minimum (min_* plus outsets, shape::fit). A
// window narrower than a skin needs grows to it when the skin is applied; a
// user's larger window is never shrunk by a skin change; and the size a skin
// grows the window to is the minimum the Host serves for it. A user's drag
// below that minimum is the OS's to refuse (the min size the shell sets);
// WebDriver's resize is no drag and macOS does not clamp it (Tauri set_size,
// r24 896e58096: 200 px kept), so what the spec drives is the shell's own
// promise: a window found below its skin's minimum is grown back to it.

import { browser, expect } from "@wdio/globals";
import { shown, switchTo, waitUntil } from "../../../walk/helpers.ts";
import { BUDDY, openSettings } from "../../../walk/nav.ts";
import { applySkin, createIdentity, newUser, type User } from "../../journeys.ts";
import { builtinPacks } from "../../skin-pack.ts";
import { treeMinimum } from "../../windows.ts";

const packName = (file: string): string => {
  const pack = builtinPacks().find((p) => p.file === file);
  if (!pack) throw new Error(`no built-in pack ${file}`);
  return pack.name;
};

async function buddySize(): Promise<{ width: number; height: number }> {
  await switchTo(browser, BUDDY);
  return browser.getWindowSize();
}

describe("a skin's window size (SK2-N4 fit)", function () {
  this.timeout(600_000);
  const me: User = newUser("winfit");

  before(async () => {
    await createIdentity(browser, me);
    await openSettings(browser, "Appearance");
  });

  it("grows the window to a wider skin, keeps a larger one, and holds its minimum", async () => {
    // Symbiosis first: the buddy list starts at its kind's size.
    await applySkin(browser, me, packName("symbiosis"));
    await switchTo(browser, BUDDY);
    await shown(browser, ".skin-tree");
    const before = await buddySize();

    // Total Chaos lays a wider frame: the window grows to it.
    await applySkin(browser, me, packName("total-chaos"));
    await waitUntil(async () => (await buddySize()).width > before.width, 10_000, "the window did not grow to Total Chaos");
    const grown = await buddySize();
    // It grew to the minimum the Host serves for Total Chaos's tree (SK4), no further.
    const served = await treeMinimum(browser);
    if (!served) throw new Error("Total Chaos lays no tree in the buddy list");
    const inner = (await browser.execute(() => ({ width: window.innerWidth, height: window.innerHeight }))) as { width: number; height: number };
    expect(inner.width).toBe(served.width);

    // The user makes the window larger still; a skin with a smaller frame keeps it.
    await switchTo(browser, BUDDY);
    await browser.setWindowSize(grown.width + 120, grown.height + 60);
    const larger = await buddySize();
    await applySkin(browser, me, packName("symbiosis"));
    await switchTo(browser, BUDDY);
    await shown(browser, ".skin-tree");
    const kept = await buddySize();
    expect(kept.width).toBeGreaterThanOrEqual(larger.width);
    expect(kept.height).toBeGreaterThanOrEqual(larger.height);

    // A window below a skin's minimum is grown back to it when that skin
    // is applied: squeezed under Symbiosis, Total Chaos restores its size.
    await switchTo(browser, BUDDY);
    await browser.setWindowSize(200, 300);
    await applySkin(browser, me, packName("total-chaos"));
    await switchTo(browser, BUDDY);
    await waitUntil(async () => (await buddySize()).width >= grown.width, 10_000, "Total Chaos did not grow a squeezed window back to its minimum");
  });
});
