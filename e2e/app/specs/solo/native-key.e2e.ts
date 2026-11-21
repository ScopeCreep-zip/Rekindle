// Plan E7.14 S5: the driver can press Tab as native input, so the rows
// that need the engine's own focus move (dialog focus traps, the buddy
// list's one tab stop, roving focus) can be tested. Proof: a Tab sent this
// way moves focus from one control to the next in Settings, and the page's
// keydown listener sees a trusted event. Where the driver cannot send
// native keys (macOS, see native-key.ts; a display GTK cannot simulate keys
// on), the test is skipped with the driver's reason printed, and those rows
// stay owed manual checks there.

import { browser, expect } from "@wdio/globals";
import { openSettings } from "../../../walk/nav.ts";
import { createIdentity, newUser } from "../../journeys.ts";
import { nativeKey } from "../../native-key.ts";

describe("native key input", function () {
  this.timeout(300_000);

  before(async () => {
    await createIdentity(browser, newUser("nativekey"));
    await openSettings(browser, "Profile");
  });

  it("a native Tab moves focus to the next control, as a trusted event", async function () {
    // Focus the first tab button and listen for the key the engine delivers.
    const start = (await browser.execute(`window.__nativeKeys = [];
document.addEventListener("keydown", function (e) { window.__nativeKeys.push({ key: e.key, trusted: e.isTrusted }); }, true);
var first = document.querySelector(".form-tab");
first.focus();
return document.activeElement === first ? first.textContent.trim() : null;`)) as string | null;
    expect(start).not.toBeNull();
    const sent = await nativeKey(browser, "Tab");
    if (sent !== "sent") {
      console.log(`[skip] native keys are not available here: ${sent.unsupported}`);
      this.skip();
    }
    let after: { focused: string; keys: { key: string; trusted: boolean }[] } = { focused: "", keys: [] };
    for (let i = 0; i < 20; i++) {
      after = (await browser.execute(
        'var a = document.activeElement; return { focused: a ? (a.textContent || a.getAttribute("aria-label") || a.tagName).trim() : "", keys: window.__nativeKeys };',
      )) as typeof after;
      if (after.keys.length > 0 && after.focused !== start) break;
      await new Promise((r) => setTimeout(r, 100));
    }
    expect(after.keys[0]).toEqual({ key: "Tab", trusted: true });
    expect(after.focused).not.toBe(start);
  });
});
