// Plan E7.7 HSC1: a community window's pack buttons act on its community.
// In Symbiosis's community window (the pack's chat frame): Profile
// (ButtonInfo) opens the community's info (its settings panel); Join
// (ButtonJoinGame) is refused in the Host's words while the community lists
// no game server; Mute (ButtonMute) is refused while we are in no voice; and
// Call (ChatButtonCall) is either refused with its reason or offered, and is
// pressed only on Linux (on macOS joining voice asks for the microphone).
// Each button's state is logged as `[hsc1]` and shot.

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, shown, switchTo, waitUntil } from "../../../walk/helpers.ts";
import { BUDDY, openSettings } from "../../../walk/nav.ts";
import { createCommunity, createIdentity, newUser, openCommunity, type User } from "../../journeys.ts";
import { applySkin, skinShown } from "../../journeys_skins.ts";
import { shot } from "../../shots.ts";
import { builtinPacks } from "../../skin-pack.ts";

const NO_SERVER = "This community lists no game server to join.";

interface ButtonState {
  shown: boolean;
  refused: boolean;
  reason: string;
  action: string;
}

/** Tree node `id` in the current window: drawn, refused (`aria-disabled`) and its Host reason. */
async function button(id: string): Promise<ButtonState> {
  return (await browser.execute((nid: string) => {
    const n = Array.from(document.querySelectorAll<HTMLElement>(`.skin-tree [data-skin-node="${nid}"]`)).find(
      (e) => !e.closest("[hidden]") && e.getBoundingClientRect().width > 0,
    );
    return n
      ? { shown: true, refused: n.getAttribute("aria-disabled") === "true", reason: n.getAttribute("aria-description") ?? "", action: n.dataset.skinAction ?? "" }
      : { shown: false, refused: false, reason: "", action: "" };
  }, id)) as ButtonState;
}

describe("a community window's Call, Mute, Join and Profile act on its community (HSC1)", function () {
  this.timeout(1_200_000);
  const me: User = { ...newUser("hsc1"), displayName: "Ari" };
  const name = `Button Hall ${Date.now() % 100000}`;

  before(async () => {
    await createIdentity(browser, me);
    const pack = builtinPacks().find((p) => p.file === "symbiosis");
    if (!pack) throw new Error("no built-in symbiosis");
    await switchTo(browser, BUDDY);
    await openSettings(browser, "Appearance");
    const href = await applySkin(browser, me, pack.name);
    await switchTo(browser, BUDDY);
    await createCommunity(browser, me, name);
    await openCommunity(browser, name);
    await skinShown(browser, href);
    await waitUntil(async () => (await browser.$('.skin-tree[data-skin-signals="live"]').isExisting().catch(() => false)), NET_WAIT_MS, "the community window's skin signals never went live");
  });

  it("Join and Mute are refused in the Host's words; Call is refused with its reason or offered", async () => {
    const [call, mute, join] = [await button("ChatButtonCall"), await button("ButtonMute"), await button("ButtonJoinGame")];
    console.log(`[hsc1] call: ${JSON.stringify(call)}`);
    console.log(`[hsc1] mute: ${JSON.stringify(mute)}`);
    console.log(`[hsc1] join: ${JSON.stringify(join)}`);
    await shot("symbiosis-community-buttons");
    expect({ shown: join.shown, refused: join.refused, reason: join.reason }).toEqual({ shown: true, refused: true, reason: NO_SERVER });
    expect({ shown: mute.shown, refused: mute.refused, hasReason: mute.reason.trim().length > 0 }).toEqual({ shown: true, refused: true, hasReason: true });
    expect(call.shown).toBe(true);
    expect(call.refused ? call.reason.trim().length > 0 : true).toBe(true);
    if (!call.refused && process.platform === "linux") {
      await (await browser.$('.skin-tree [data-skin-node="ChatButtonCall"]')).click();
      await waitUntil(async () => (await button("ChatButtonCall")).shown === false || (await button("ChatButtonLeaveCall")).shown, NET_WAIT_MS, "Call did not join the community's voice");
      console.log("[hsc1] call pressed: in voice");
    } else if (!call.refused) {
      console.log("[hsc1] call offered, not pressed on macOS (joining voice asks for the microphone)");
    }
  });

  it("Profile opens the community's info", async () => {
    const info = await button("ButtonInfo");
    console.log(`[hsc1] profile: ${JSON.stringify(info)}`);
    expect({ shown: info.shown, refused: info.refused }).toEqual({ shown: true, refused: false });
    await (await browser.$('.skin-tree [data-skin-node="ButtonInfo"]')).click();
    await shown(browser, ".modal-container", NET_WAIT_MS);
    console.log("[hsc1] profile opened the community's settings panel");
    await shot("symbiosis-community-info");
  });
});
