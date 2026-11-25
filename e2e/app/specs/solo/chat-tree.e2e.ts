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

const PACKS = ["onyx", "xslimfire", "symbiosis", "spacecowboy", "stealthx", "alloy", "simple-x", "combat"];
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

/**
 * Lead's ask (Onyx's three grey bars under the menu titles): every tree node
 * in the top 140 px whose box is a thin strip (1-14 px high), with its id,
 * role, box and art (background image, colour), as `[strips]`; and the
 * chatTitle's box, visibility, opacity and colour as `[title]`.
 */
/**
 * Lead (r31: Symbiosis's DM showed "Answer" lit with no call): the signal set
 * the Host served this window (`data-skin-signal-on`, `data-skin-cannot` on
 * `.skin-tree`), and for each voice node its drawn state, whether disabled,
 * its index among its parent's children (a control group's page) and the
 * parent; plus the node behind any drawn "Answer" word. One line, `[voice]`.
 */
async function voiceEvidence(): Promise<string> {
  return (await browser.execute(() => {
    const tree = document.querySelector<HTMLElement>(".skin-tree");
    const drawn = (e: Element | null): boolean => !!e && !e.closest("[hidden]") && e.getBoundingClientRect().width > 0;
    const nodes = ["ChatButtonCall", "ChatButtonAnswerCall", "ChatButtonLeaveCall"].map((id) => {
      const n = document.querySelector<HTMLElement>(`[data-skin-node="${id}"]`);
      if (!n) return `${id}: absent`;
      const parent = n.parentElement?.closest<HTMLElement>("[data-skin-node]");
      const siblings = parent ? Array.from(parent.children).filter((c) => (c as HTMLElement).dataset.skinNode) : [];
      return `${id}: ${drawn(n) ? "drawn" : "hidden"}${n.getAttribute("aria-disabled") === "true" ? " disabled" : ""} index ${siblings.indexOf(n)} of ${parent?.dataset.skinNode ?? "?"}`;
    });
    const answer = Array.from(document.querySelectorAll<HTMLElement>(".skin-node-label-text")).find((t) => drawn(t) && /answer/i.test(t.textContent ?? ""));
    return [
      `signals on=[${tree?.dataset.skinSignalOn ?? ""}] cannot=[${tree?.dataset.skinCannot ?? ""}]`,
      ...nodes,
      `"Answer" word on ${answer?.closest<HTMLElement>("[data-skin-node]")?.dataset.skinNode ?? "none"}`,
    ].join("; ");
  })) as string;
}

async function stripsAndTitle(): Promise<{ strips: string; title: string }> {
  return (await browser.execute(() => {
    const box = (b: DOMRect): string => `${Math.round(b.left)},${Math.round(b.top)} ${Math.round(b.width)}x${Math.round(b.height)}`;
    const strips = Array.from(document.querySelectorAll<HTMLElement>(".chat-window [data-skin-node]"))
      .map((e) => ({ e, b: e.getBoundingClientRect() }))
      .filter(({ b }) => b.top < 140 && b.height >= 1 && b.height <= 14 && b.width > 0)
      .map(({ e, b }) => {
        const cs = getComputedStyle(e);
        const art = cs.backgroundImage === "none" ? "no art" : cs.backgroundImage.replace(/^url\("?|"?\)$/g, "").split("/").slice(-2).join("/");
        return `${e.dataset.skinNode} role=${e.dataset.skinRole ?? "-"} [${box(b)}] art=${art.slice(0, 70)} bg=${cs.backgroundColor}`;
      });
    const t = document.querySelector<HTMLElement>(".skin-chat-title");
    let title = "no .skin-chat-title";
    if (t) {
      const cs = getComputedStyle(t);
      const parent = t.parentElement;
      title = `"${(t.textContent ?? "").trim()}" [${box(t.getBoundingClientRect())}] visibility=${cs.visibility} opacity=${cs.opacity} color=${cs.color} font=${cs.fontSize}; node ${parent?.dataset.skinNode ?? "-"} [${parent ? box(parent.getBoundingClientRect()) : ""}] overflow=${parent ? getComputedStyle(parent).overflow : ""}`;
    }
    return { strips: strips.join(" | ") || "none", title };
  })) as { strips: string; title: string };
}

/**
 * Lead (CHT1, Linux): where the frame's parts sit, not only what they read.
 * (1) The title text lies inside the pack's title strip (its titleBar node,
 * else its chatTitle node) and inside the pack window, never at the window
 * corner; (2) in a 1:1 window the presence row (`.chat-peer-status`) is drawn
 * inside the chat area; (3) no frame tile (a decor node) reaches outside the
 * window. The problems, as `[cht1]` lines; empty when all hold.
 */
async function chatFrameChecks(kind: "dm" | "group"): Promise<string[]> {
  return (await browser.execute((k: string) => {
    const out: string[] = [];
    const drawn = (e: Element | null): e is HTMLElement => !!e && !e.closest("[hidden]") && e.getBoundingClientRect().width > 0;
    const box = (e: Element): string => {
      const r = e.getBoundingClientRect();
      return `${Math.round(r.left)},${Math.round(r.top)} ${Math.round(r.width)}x${Math.round(r.height)}`;
    };
    const inside = (a: DOMRect, b: DOMRect): boolean => a.left >= b.left - 1 && a.right <= b.right + 1 && a.top >= b.top - 1 && a.bottom <= b.bottom + 1;
    const tree = document.querySelector(".chat-window .skin-tree");
    const text = document.querySelector(".chat-window .skin-chat-title");
    if (!tree || !drawn(text)) return ["no drawn chat title"];
    const strip =
      Array.from(document.querySelectorAll<HTMLElement>('.skin-tree [data-skin-role="titleBar"]')).find(drawn) ??
      text.closest<HTMLElement>('[data-skin-role="chatTitle"]');
    const t = text.getBoundingClientRect();
    if (!strip) out.push(`title ${box(text)} has no title strip`);
    else if (!inside(t, strip.getBoundingClientRect())) out.push(`title ${box(text)} is outside its strip ${strip.dataset.skinNode} ${box(strip)}`);
    if (!inside(t, tree.getBoundingClientRect())) out.push(`title ${box(text)} is outside the pack window ${box(tree)}`);
    if (k === "dm") {
      const row = document.querySelector(".chat-peer-status");
      const area = document.querySelector('[data-skin-role="slot:chatArea"]');
      if (!drawn(row)) out.push("no drawn presence row");
      else if (area && !inside(row.getBoundingClientRect(), area.getBoundingClientRect())) out.push(`presence row ${box(row)} is outside the chat area ${box(area)}`);
    }
    for (const n of Array.from(document.querySelectorAll<HTMLElement>('.skin-tree [data-skin-role="decor"]')).filter(drawn)) {
      const r = n.getBoundingClientRect();
      if (r.left < -0.5 || r.top < -0.5 || r.right > innerWidth + 0.5 || r.bottom > innerHeight + 0.5) out.push(`tile ${n.dataset.skinNode} ${box(n)} is clipped by the window ${innerWidth}x${innerHeight}`);
    }
    return out;
  }, kind)) as string[];
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
        // Evidence first, so a failing check below still leaves it.
        await browser.pause(2000);
        console.log(`[slot] ${file} ${kind}: ${await slotBoxes()}`);
        const st = await stripsAndTitle();
        console.log(`[strips] ${file} ${kind}: ${st.strips}`);
        console.log(`[title] ${file} ${kind}: ${st.title}`);
        console.log(`[voice] ${file} ${kind}: ${await voiceEvidence()}`);
        await shot(`${kind}-before-checks`);
        await treeLaid(handle, title, text);
        const def = await sizeTo(CHAT_DM.def.width, CHAT_DM.def.height);
        await treeLaid(handle, title, text);
        await shot(`${kind}-default-${def}`);
        const frame = await chatFrameChecks(kind);
        console.log(`[cht1] ${file} ${kind}: ${frame.length === 0 ? "title in its strip, presence row drawn, no clipped tile" : JSON.stringify(frame)}`);
        expect({ pack: file, window: kind, problems: frame }).toEqual({ pack: file, window: kind, problems: [] });
        const min = await sizeTo(CHAT_DM.min.width, CHAT_DM.min.height);
        await treeLaid(handle, title, text);
        await shot(`${kind}-min-${min}`);
        expect(min).not.toBe("");
      }
    });
  }
});
