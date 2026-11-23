// Plan E7.2 A (contract A): the skin's Show Voice with neither a call nor a
// voice session. The Host answers 409 "ShowVoiceWindow: no call or voice
// session"; the button stays (no capability), and the window shows that
// answer. No faithful pack lays a Show Voice button, so a pack derived from
// Symbiosis (skin-pack.ts showVoicePack) is imported through Settings →
// Import, the real library path. The call and voice cases open the
// microphone, so they run on Linux (specs/devices/show-voice.e2e.ts).

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, press, shown, switchTo, waitUntil } from "../../../walk/helpers.ts";
import { BUDDY, SETTINGS, openSettings } from "../../../walk/nav.ts";
import { createIdentity, friendsView, importAndKeepSkin, newUser, skinShown, type User } from "../../journeys.ts";
import { SHOW_VOICE_PACK, showVoicePack } from "../../skin-pack.ts";

describe("Show Voice with no call or voice session", function () {
  this.timeout(600_000);
  const me: User = newUser("showvoice");

  it("the button stays, and the window shows the Host's answer", async () => {
    await createIdentity(browser, me);
    await openSettings(browser, "Appearance");
    const href = await importAndKeepSkin(browser, me, showVoicePack(), SHOW_VOICE_PACK.name);
    await switchTo(browser, BUDDY);
    await skinShown(browser, href);
    await friendsView(browser);
    const node = `[data-skin-node="${SHOW_VOICE_PACK.node}"]`;
    const button = await shown(browser, node, NET_WAIT_MS);
    expect(await button.getAttribute("aria-disabled")).not.toBe("true");
    await press(browser, button);
    // Every toast the window shows, so a failure names what it said instead.
    let said = "";
    await waitUntil(
      async () => {
        said = ((await browser.execute(() =>
          Array.from(document.querySelectorAll(".toast-item")).map((t) => `${t.className}: ${(t.textContent ?? "").trim()}`).join(" | "),
        )) as string);
        return /no call or voice session/.test(said);
      },
      NET_WAIT_MS,
      "the Host's answer",
    ).catch(() => {
      throw new Error(`the window never showed the Host's answer; its toasts: ${said || "none"}`);
    });
    expect(said).toContain("no call or voice session");
    // Still there and pressable: the window says so, the button stays.
    expect(await (await browser.$(node)).isDisplayed()).toBe(true);
    // The derived skin is still the one in use (its trial was kept).
    await switchTo(browser, SETTINGS);
    expect((await (await browser.$(".skin-row-shown .skin-name")).getText()).trim()).toBe(SHOW_VOICE_PACK.name);
  });
});
