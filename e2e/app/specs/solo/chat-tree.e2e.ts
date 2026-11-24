// Plan SK17-2: a pack that lays its chat window as a tree draws it whole
// (frame, title bar, window buttons, menu row), the conversation title in its
// chatTitle node, and the chat's own parts in its slots: the message list in
// chatArea, the composer in chatEdit. A 1:1 window (with Bea, a fleet friend
// who wrote first) and a group window (Bea and Cara) in each pack that lays
// one (the XML packs: Onyx, xSLIMFIRE, Symbiosis, SpaceCowboy, StealthX), at
// the shell's default and minimum chat sizes; each state a shot.

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, doubleClick, shown, switchTo, waitForHandle, waitUntil } from "../../../walk/helpers.ts";
import { BUDDY, openSettings } from "../../../walk/nav.ts";
import { type Peer, peerAcceptGroup, peerSendDm, startFleet, stopFleet } from "../../fleet.ts";
import { applySkin, createGroup, createIdentity, friendRow, newUser, openGroup, seesMessage, skinShown, type User } from "../../journeys.ts";
import { shot } from "../../shots.ts";
import { builtinPacks } from "../../skin-pack.ts";
import { befriendFleet } from "../../social.ts";
import { CHAT_DM } from "../../windows.ts";

const PACKS = ["onyx", "xslimfire", "symbiosis", "spacecowboy", "stealthx"];
const GROUP = "Night Raid";

/** The chat window `handle` draws the pack's tree with `title` in its chatTitle node and the chat in its slots. */
async function treeLaid(handle: string, title: string): Promise<void> {
  await browser.switchToWindow(handle);
  let last = "";
  await waitUntil(
    async () => {
      last = (await browser.execute((want: string) => {
        const w = document.querySelector(".chat-window");
        if (!w?.querySelector(".skin-tree")) return "no chat tree";
        const t = w.querySelector(".skin-chat-title")?.textContent?.trim() ?? "";
        if (t !== want) return `chatTitle reads "${t}"`;
        if (!w.querySelector('[data-skin-role="slot:chatEdit"] .message-input-field')) return "the composer is not in chatEdit";
        if (!w.querySelector('[data-skin-role="slot:chatArea"] .chat-message-area')) return "the messages are not in chatArea";
        return "";
      }, title)) as string;
      return last === "";
    },
    NET_WAIT_MS,
    `the chat tree for ${title}`,
  ).catch(() => {
    throw new Error(`chat window for ${title}: ${last}`);
  });
}

/** Size the current window to `width`x`height` (the OS keeps it at the skin's minimum if that is larger); its inner size. */
async function sizeTo(width: number, height: number): Promise<string> {
  await browser.setWindowSize(width, height);
  const inner = (await browser.execute(() => `${innerWidth}x${innerHeight}`)) as string;
  return inner;
}

describe("a pack's chat tree draws the chat window, the chat in its slots", function () {
  this.timeout(3_600_000);
  const me: User = { ...newUser("chattree"), displayName: "Ari" };
  let peers: Peer[] = [];
  let dm = "";
  let group = "";

  before(async () => {
    await createIdentity(browser, me);
    peers = await startFleet(["Bea", "Cara"]);
    const myKey = await befriendFleet(browser, me, peers);
    await peerSendDm(peers[0], myKey, "hey, up for a game tonight?");
    const before = await browser.getWindowHandles();
    await doubleClick(browser, await friendRow(browser, "Bea"));
    dm = await waitForHandle(browser, (h) => h.startsWith("chat-") && !before.includes(h));
    await browser.switchToWindow(dm);
    await seesMessage(browser, me, "hey, up for a game");
    await switchTo(browser, BUDDY);
    await createGroup(browser, me, GROUP, ["Bea", "Cara"]);
    await Promise.all(peers.map((p) => peerAcceptGroup(p, GROUP)));
    await openGroup(browser, GROUP);
    group = await browser.getWindowHandle();
    await openSettings(browser, "Appearance");
  });

  after(async () => {
    await stopFleet(peers);
  });

  for (const file of PACKS) {
    it(`${file}: the 1:1 and group windows draw the pack's chat tree at default and minimum size`, async () => {
      const pack = builtinPacks().find((p) => p.file === file);
      if (!pack) throw new Error(`no built-in pack ${file}`);
      const href = await applySkin(browser, me, pack.name);
      for (const [handle, title, kind] of [
        [dm, "Bea", "dm"],
        [group, GROUP, "group"],
      ] as const) {
        await browser.switchToWindow(handle);
        await skinShown(browser, href);
        await treeLaid(handle, title);
        const def = await sizeTo(CHAT_DM.def.width, CHAT_DM.def.height);
        await treeLaid(handle, title);
        await shot(`${kind}-default-${def}`);
        const min = await sizeTo(CHAT_DM.min.width, CHAT_DM.min.height);
        await treeLaid(handle, title);
        await shot(`${kind}-min-${min}`);
        expect(min).not.toBe("");
      }
      await openSettings(browser, "Appearance");
    });
  }
});
