// SK16 capture (the lead, 2026-11-23): each built-in skin's Community window
// (a channel open, the member list showing a peer) beside the same skin's
// buddy list and a DM chat window, as <pack>-community.png, <pack>-main.png
// and <pack>-chat.png in $SK16_OUT (default e2e/app/out/sk16). A capture, not
// a check; it opens no microphone, so it runs anywhere under wdio.conf.ts:
//   SK16_OUT=<dir> pnpm exec wdio run e2e/app/wdio.conf.ts \
//     --spec e2e/app/specs/capture/sk16.e2e.ts
import { mkdirSync } from "node:fs";
import path from "node:path";
import { browser } from "@wdio/globals";
import { NET_WAIT_MS, doubleClick, shown, sleep, switchTo, waitForHandle, waitUntil } from "../../../walk/helpers.ts";
import { BUDDY, openSettings } from "../../../walk/nav.ts";
import { type Peer, peerJoinCommunity, peerSendDm, startFleet, stopFleet } from "../../fleet.ts";
import {
  applySkin,
  createCommunity,
  createIdentity,
  createInvite,
  friendRow,
  newUser,
  openCommunity,
  postMessage,
  seesMessage,
  skinShown,
  type User,
} from "../../journeys.ts";
import { builtinPacks } from "../../skin-pack.ts";
import { befriendFleet } from "../../social.ts";

const OUT = process.env.SK16_OUT ?? path.join(import.meta.dirname, "..", "..", "out", "sk16");

describe("SK16 capture", function () {
  this.timeout(3_600_000);
  const me: User = { ...newUser("sk16"), displayName: "Ari" };
  let peers: Peer[] = [];
  let chat = "";
  let community = "";

  after(async () => {
    await stopFleet(peers);
  });

  it("captures each skin's buddy list, chat and Community windows", async () => {
    mkdirSync(OUT, { recursive: true });
    await createIdentity(browser, me);
    peers = await startFleet(["Bea"]);
    const myKey = await befriendFleet(browser, me, peers);
    await peerSendDm(peers[0], myKey, "hey, up for a game tonight?");
    const before = await browser.getWindowHandles();
    await doubleClick(browser, await friendRow(browser, "Bea"));
    chat = await waitForHandle(browser, (h) => h.startsWith("chat-") && !before.includes(h));
    await browser.switchToWindow(chat);
    await shown(browser, ".message-input-field");
    await seesMessage(browser, me, "hey, up for a game");
    await switchTo(browser, BUDDY);
    await createCommunity(browser, me, "Night Raid");
    await openCommunity(browser, "Night Raid");
    community = await browser.getWindowHandle();
    const invite = await createInvite(browser, me, 1);
    await peerJoinCommunity(peers[0], invite);
    await waitUntil(async () => (await browser.$$(".member-item")).length >= 2, NET_WAIT_MS, "the member list never held the peer");
    await postMessage(browser, me, "gg everyone, same time tomorrow?");
    await openSettings(browser, "Appearance");
    for (const pack of builtinPacks()) {
      const href = await applySkin(browser, me, pack.name);
      const shots: [string, string][] = [
        ["main", ""],
        ["chat", chat],
        ["community", community],
      ];
      for (const [kind, handle] of shots) {
        if (handle) await browser.switchToWindow(handle);
        else await switchTo(browser, BUDDY);
        await skinShown(browser, href);
        await sleep(600);
        await browser.saveScreenshot(path.join(OUT, `${pack.file}-${kind}.png`));
      }
      console.log(`SK16 captured ${pack.file}`);
    }
  });
});
