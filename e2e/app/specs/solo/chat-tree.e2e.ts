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

/** The chatArea slot's box, each child's (the portals' wrappers), the list's and each message's (`[slot]` evidence). */
async function slotBoxes(): Promise<string> {
  return (await browser.execute(() => {
    const r = (e: Element | null): string => {
      if (!e) return "none";
      const b = e.getBoundingClientRect();
      return `${Math.round(b.left)},${Math.round(b.top)} ${Math.round(b.width)}x${Math.round(b.height)}`;
    };
    const slot = document.querySelector('[data-skin-role="slot:chatArea"]');
    const kids = slot ? Array.from(slot.children).map((c) => `${c.tagName.toLowerCase()}.${Array.from(c.classList).join(".")}[${r(c)} ${getComputedStyle(c).display}]`) : [];
    const msgs = Array.from(document.querySelectorAll(".chat-message")).map((m) => r(m));
    return `slot ${r(slot)} overflow=${slot ? getComputedStyle(slot).overflow : ""}; children ${kids.join(" | ")}; list ${r(document.querySelector(".chat-message-area"))}; messages ${msgs.join(" ; ") || "none"}`;
  })) as string;
}

/** The chat window `handle` draws the pack's tree with `title` in its chatTitle node and the chat in its slots; `text`, when given, is a message drawn inside the chatArea slot's box. */
async function treeLaid(handle: string, title: string, text?: string): Promise<void> {
  await browser.switchToWindow(handle);
  let last = "";
  await waitUntil(
    async () => {
      last = (await browser.execute((want: string, seen: string) => {
        const w = document.querySelector(".chat-window");
        if (!w?.querySelector(".skin-tree")) return "no chat tree";
        const title = w.querySelector<HTMLElement>(".skin-chat-title");
        const t = title?.textContent?.trim() ?? "";
        if (t !== want) return `chatTitle reads "${t}"`;
        // Drawn, not only present: a box inside the window, not hidden or transparent.
        const tb = title!.getBoundingClientRect();
        const tcs = getComputedStyle(title!);
        if (tb.width < 4 || tb.height < 4 || tb.right <= 0 || tb.left >= innerWidth || tb.bottom <= 0 || tb.top >= innerHeight) return `chatTitle has no visible box (${Math.round(tb.left)},${Math.round(tb.top)} ${Math.round(tb.width)}x${Math.round(tb.height)})`;
        if (tcs.visibility !== "visible" || tcs.opacity === "0" || tcs.color === "rgba(0, 0, 0, 0)") return `chatTitle is not drawn (visibility ${tcs.visibility}, opacity ${tcs.opacity}, color ${tcs.color})`;
        if (!w.querySelector('[data-skin-role="slot:chatEdit"] .message-input-field')) return "the composer is not in chatEdit";
        if (!w.querySelector('[data-skin-role="slot:chatArea"] .chat-message-area')) return "the messages are not in chatArea";
        if (seen) {
          const slot = w.querySelector('[data-skin-role="slot:chatArea"]')!.getBoundingClientRect();
          const msg = Array.from(w.querySelectorAll(".chat-message")).find((m) => (m.textContent ?? "").includes(seen));
          if (!msg) return `no message "${seen}"`;
          const b = msg.getBoundingClientRect();
          if (b.width === 0 || b.height === 0) return `"${seen}" has no box`;
          if (b.top < slot.top || b.bottom > slot.bottom + 1 || b.left < slot.left || b.right > slot.right + 1) return `"${seen}" is outside the chatArea slot`;
        }
        return "";
      }, title, text ?? "")) as string;
      return last === "";
    },
    NET_WAIT_MS,
    `the chat tree for ${title}`,
  ).catch(async () => {
    throw new Error(`chat window for ${title}: ${last} [slot] ${await slotBoxes()}`);
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
      for (const [handle, title, kind, text] of [
        [dm, "Bea", "dm", "hey, up for a game"],
        [group, GROUP, "group", undefined],
      ] as const) {
        await browser.switchToWindow(handle);
        await skinShown(browser, href);
        await treeLaid(handle, title, text);
        console.log(`[slot] ${file} ${kind}: ${await slotBoxes()}`);
        const def = await sizeTo(CHAT_DM.def.width, CHAT_DM.def.height);
        await treeLaid(handle, title, text);
        await shot(`${kind}-default-${def}`);
        const min = await sizeTo(CHAT_DM.min.width, CHAT_DM.min.height);
        await treeLaid(handle, title, text);
        await shot(`${kind}-min-${min}`);
        expect(min).not.toBe("");
      }
    });
  }
});
