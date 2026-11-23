// SK16 capture (the lead, 2026-11-23): each built-in skin's Community window
// with the user in a voice channel, so the voice bar and the connected-members
// row show. Saved as <pack>-community-voice.png in $SK16_OUT (default
// e2e/app/out/sk16). A capture, not a check: it asserts only that the voice
// panel showed. Needs the microphone, so it runs on Linux under
// wdio.devices.conf.ts; on macOS it refuses before joining (a TCC check
// nothing here may trigger):
//   SK16_OUT=<dir> pnpm exec wdio run e2e/app/wdio.devices.conf.ts \
//     --spec e2e/app/specs/capture/sk16-voice.e2e.ts
import { mkdirSync } from "node:fs";
import path from "node:path";
import { browser } from "@wdio/globals";
import { NET_WAIT_MS, WAIT_MS, byText, gone, resolveIntent, shown, sleep, switchTo, typeInto, waitUntil } from "../../../walk/helpers.ts";
import { BUDDY, openSettings, setValue } from "../../../walk/nav.ts";
import { type Peer, peerJoinCommunity, startFleet, stopFleet } from "../../fleet.ts";
import { applySkin, createCommunity, createIdentity, createInvite, newUser, openCommunity, skinShown, type User } from "../../journeys.ts";
import { builtinPacks } from "../../skin-pack.ts";
import { befriendFleet } from "../../social.ts";

const OUT = process.env.SK16_OUT ?? path.join(import.meta.dirname, "..", "..", "out", "sk16");
const VOICE = "Lounge";

describe("SK16 capture: the Community window in a voice channel", function () {
  this.timeout(3_600_000);
  const me: User = { ...newUser("sk16v"), displayName: "Ari" };
  let peers: Peer[] = [];

  after(async () => {
    await stopFleet(peers);
  });

  it("captures each skin's Community window with the voice bar", async () => {
    if (process.platform === "darwin") throw new Error("macOS: joining voice opens the microphone (a TCC check); run this on Linux");
    mkdirSync(OUT, { recursive: true });
    await createIdentity(browser, me);
    peers = await startFleet(["Bea"]);
    await befriendFleet(browser, me, peers);
    await createCommunity(browser, me, "Night Raid");
    await openCommunity(browser, "Night Raid");
    const community = await browser.getWindowHandle();
    await peerJoinCommunity(peers[0], await createInvite(browser, me, 1));
    await waitUntil(async () => (await browser.$$(".member-item")).length >= 2, NET_WAIT_MS, "the member list never held the peer");
    // A voice channel, as the walk makes one (rows-voice.ts M10v), then joined
    // from the call stage's Join button (CallStage.tsx).
    await (await shown(browser, '[title="Create Channel"]')).click();
    await typeInto(await shown(browser, 'input[placeholder="Channel name..."]'), VOICE);
    await setValue(browser, ".modal-container select.form-select", "voice");
    await (await browser.$('.modal-container button[type="submit"]')).click();
    await resolveIntent(browser, { passphrase: me.passphrase, appearMs: 3000 });
    await gone(browser, 'input[placeholder="Channel name..."]', NET_WAIT_MS);
    await (await byText(browser, ".channel-item", VOICE, NET_WAIT_MS)).click();
    await (await shown(browser, ".call-stage-join-btn", WAIT_MS)).click();
    await shown(browser, ".voice-panel", NET_WAIT_MS);
    await openSettings(browser, "Appearance");
    for (const pack of builtinPacks()) {
      const href = await applySkin(browser, me, pack.name);
      await browser.switchToWindow(community);
      await skinShown(browser, href);
      await shown(browser, ".voice-panel");
      await sleep(600);
      await browser.saveScreenshot(path.join(OUT, `${pack.file}-community-voice.png`));
      console.log(`SK16 voice captured ${pack.file}`);
      await switchTo(browser, BUDDY);
    }
  });
});
