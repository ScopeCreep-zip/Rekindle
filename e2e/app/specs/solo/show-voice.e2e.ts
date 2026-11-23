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
import { shot } from "../../shots.ts";

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
    // The answer is seen, not only in the DOM: the toast is inside the window
    // and on top at its centre (a shot of the first run showed no toast).
    // Seen means once its entrance has run (the slide-in starts at opacity 0,
    // 20 px right): the toast is then inside the window, opaque, and on top
    // at its centre. The first run measured its first frame (outside by the
    // 20 px, and the shot showed no toast), so the report names the
    // animation's state and the toast's age.
    const seen = (await browser.execute(() => new Promise<string>((done) => {
      const toast = Array.from(document.querySelectorAll<HTMLElement>(".toast-item")).find((t) => /no call or voice session/.test(t.textContent ?? ""));
      if (!toast) return done("no toast");
      const settle = Promise.all(toast.getAnimations().map((a) => a.finished));
      const timeout = new Promise((r) => setTimeout(r, 2_000));
      void Promise.race([settle.then(() => "finished"), timeout.then(() => "unfinished")]).then((ran) => {
        const anims = toast.getAnimations().map((a) => `${a.playState}@${Math.round(Number(a.currentTime ?? -1))}`).join(",");
        const opacity = getComputedStyle(toast).opacity;
        const r = toast.getBoundingClientRect();
        const state = `animation ${ran} [${anims}], opacity ${opacity}, at ${Math.round(r.left)},${Math.round(r.top)} ${Math.round(r.width)}x${Math.round(r.height)} in ${innerWidth}x${innerHeight}`;
        if (ran !== "finished" || opacity !== "1") return done(`not shown: ${state}`);
        if (r.width === 0 || r.bottom > innerHeight || r.right > innerWidth || r.left < 0 || r.top < 0) return done(`outside the window: ${state}`);
        const top = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
        done(top && toast.contains(top) ? "" : `covered by ${top ? ((top as HTMLElement).dataset.skinNode ?? top.className) : "nothing"}: ${state}`);
      });
    }))) as string;
    // The asserted state: the derived skin's buddy list with the Host's answer showing.
    await shot("host-answer");
    expect(seen).toBe("");
    // Still there and pressable: the window says so, the button stays.
    expect(await (await browser.$(node)).isDisplayed()).toBe(true);
    // The derived skin is still the one in use (its trial was kept).
    await switchTo(browser, SETTINGS);
    expect((await (await browser.$(".skin-row-shown .skin-name")).getText()).trim()).toBe(SHOW_VOICE_PACK.name);
  });
});
