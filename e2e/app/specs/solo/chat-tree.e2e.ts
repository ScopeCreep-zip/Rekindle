// Plan SK17-2: a pack that lays its chat window as a tree draws it whole
// (frame, title bar, window buttons, menu row), the conversation title in its
// chatTitle node, and the chat's own parts in its slots: the message list in
// chatArea, the composer in chatEdit. A 1:1 window (with Bea, a fleet friend
// who wrote first) and a group window (Bea and Cara) in each pack that lays
// one (the XML packs: Onyx, xSLIMFIRE, Symbiosis, SpaceCowboy, StealthX), at
// the shell's default and minimum chat sizes; each state a shot.

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, byText, doubleClick, press, shown, switchTo, waitForHandle, waitUntil } from "../../../walk/helpers.ts";
import { BUDDY, SETTINGS, openSettings } from "../../../walk/nav.ts";
import { type Peer, peerAcceptGroup, peerSendDm, startFleet, stopFleet } from "../../fleet.ts";
import { applySkin, createGroup, createIdentity, friendRow, friendsView, newUser, openGroup, seesMessage, skinShown, type User } from "../../journeys.ts";
import { decodePng } from "../../png.ts";
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
/**
 * Lead (CW1, `9c50e1aee`): with no call ringing, an idle 1:1 window's
 * served set holds `voiceChat` (a pack's Answer) in `cannot`, and where the
 * pack lays a Call/Answer group the Call is the drawn page, the Answer not.
 */
async function idleDmOffersCall(file: string): Promise<void> {
  const st = (await browser.execute(() => {
    const tree = document.querySelector<HTMLElement>(".skin-tree");
    const drawn = (id: string): boolean | null => {
      const n = document.querySelector<HTMLElement>(`[data-skin-node="${id}"]`);
      if (!n) return null;
      return !n.closest("[hidden]") && n.getBoundingClientRect().width > 0;
    };
    return {
      on: (tree?.dataset.skinSignalOn ?? "").split(" ").filter(Boolean),
      cannot: (tree?.dataset.skinCannot ?? "").split(" ").filter(Boolean),
      call: drawn("ChatButtonCall"),
      answer: drawn("ChatButtonAnswerCall"),
    };
  })) as { on: string[]; cannot: string[]; call: boolean | null; answer: boolean | null };
  console.log(`[cw1] ${file} idle dm: ${JSON.stringify(st)}`);
  expect(st.on).not.toContain("voiceRinging");
  expect(st.cannot).toContain("voiceChat");
  if (st.call !== null) expect(st.call).toBe(true);
  if (st.answer !== null) expect(st.answer).toBe(false);
}

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
 * Lead (CHT1, Linux): where the frame's parts sit and that they are seen. A
 * node under the pack's art has a box but is not seen (the control's Alloy
 * and Simple X), and `elementFromPoint` cannot tell: it hit-tests, and the
 * pack's transparent drag and resize regions sit over visible parts. So a
 * part is seen when hiding it changes the window's pixels inside its box.
 * (1) The title's glyphs (a Range over its text) lie inside its chatTitle
 * node, ±3 px, and are seen; (2) in a 1:1 window the presence row, the
 * friend's message and the composer are each seen; (3) no node that paints
 * art reaches outside the window. The problems, as `[cht1]` lines; empty
 * when all hold.
 */
async function chatFrameChecks(kind: "dm" | "group", message: string): Promise<string[]> {
  const found = (await browser.execute((k: string, msg: string) => {
    const out: string[] = [];
    const drawn = (e: Element | null): e is HTMLElement => !!e && !e.closest("[hidden]") && e.getBoundingClientRect().width > 0;
    const rect = (r: DOMRect): [number, number, number, number] => [r.left, r.top, r.width, r.height];
    const fmt = (r: DOMRect): string => `${Math.round(r.left)},${Math.round(r.top)} ${Math.round(r.width)}x${Math.round(r.height)}`;
    const inside = (a: DOMRect, b: DOMRect): boolean => a.left >= b.left - 3 && a.right <= b.right + 3 && a.top >= b.top - 3 && a.bottom <= b.bottom + 3;
    const parts: { name: string; selector: string; box: [number, number, number, number] }[] = [];
    const text = document.querySelector(".chat-window .skin-chat-title");
    if (!document.querySelector(".chat-window .skin-tree") || !drawn(text)) return { out: ["no drawn chat title"], parts };
    const range = document.createRange();
    range.selectNodeContents(text);
    const glyphs = range.getBoundingClientRect();
    const strip = text.closest<HTMLElement>('[data-skin-role="chatTitle"]');
    if (glyphs.width === 0) out.push("the title has no glyphs");
    else {
      if (!strip) out.push(`title ${fmt(glyphs)} has no chatTitle node`);
      else if (!inside(glyphs, strip.getBoundingClientRect())) out.push(`title ${fmt(glyphs)} is outside its ${strip.dataset.skinNode} ${fmt(strip.getBoundingClientRect())}`);
      parts.push({ name: "title", selector: ".chat-window .skin-chat-title", box: rect(glyphs) });
    }
    if (k === "dm") {
      const message = Array.from(document.querySelectorAll(".chat-message")).find((m) => (m.textContent ?? "").includes(msg));
      if (message) message.setAttribute("data-cht1-message", "");
      for (const [name, selector] of [
        ["presence row", ".chat-peer-status"],
        ["message", "[data-cht1-message]"],
        ["composer", ".message-input-field"],
      ] as const) {
        const e = document.querySelector(selector);
        if (!drawn(e)) out.push(`no drawn ${name}`);
        else parts.push({ name, selector, box: rect(e.getBoundingClientRect()) });
      }
    }
    if (k === "dm") {
      // Lead (CHT1): the presence row is the conversation header, under the
      // menu and above the first message, inside the chat area's top.
      const row = document.querySelector(".chat-peer-status");
      const first = document.querySelector(".chat-message");
      const area = row?.closest('[data-skin-role="slot:chatArea"]');
      if (drawn(row) && drawn(first)) {
        const r = row.getBoundingClientRect();
        const m = first.getBoundingClientRect();
        if (r.bottom > m.top + 1) out.push(`presence row ${fmt(r)} is not above the first message ${fmt(m)}`);
        if (area && r.top < area.getBoundingClientRect().top - 1) out.push(`presence row ${fmt(r)} starts above its chat area ${fmt(area.getBoundingClientRect())}`);
      }
    }
    for (const n of Array.from(document.querySelectorAll<HTMLElement>(".skin-tree [data-skin-node]")).filter(drawn)) {
      if (getComputedStyle(n).backgroundImage === "none") continue;
      const r = n.getBoundingClientRect();
      if (r.left < -0.5 || r.top < -0.5 || r.right > innerWidth + 0.5 || r.bottom > innerHeight + 0.5) out.push(`art ${n.dataset.skinNode} ${fmt(r)} is clipped by the window ${innerWidth}x${innerHeight}`);
    }
    return { out, parts };
  }, kind, message)) as { out: string[]; parts: { name: string; selector: string; box: [number, number, number, number] }[] };
  const width = (await browser.execute(() => innerWidth)) as number;
  const before = decodePng(Buffer.from(await browser.takeScreenshot(), "base64"));
  const scale = before.width / width;
  for (const part of found.parts) {
    await browser.execute((sel: string) => document.querySelector<HTMLElement>(sel)?.style.setProperty("visibility", "hidden"), part.selector);
    const after = decodePng(Buffer.from(await browser.takeScreenshot(), "base64"));
    await browser.execute((sel: string) => document.querySelector<HTMLElement>(sel)?.style.removeProperty("visibility"), part.selector);
    const [x, y, w, h] = part.box.map((v) => Math.round(v * scale));
    let changed = 0;
    for (let row = Math.max(0, y); row < Math.min(before.height, y + h); row++) {
      for (let col = Math.max(0, x); col < Math.min(before.width, x + w); col++) {
        const i = (row * before.width + col) * 4;
        if (Math.abs(before.data[i] - after.data[i]) + Math.abs(before.data[i + 1] - after.data[i + 1]) + Math.abs(before.data[i + 2] - after.data[i + 2]) > 48) changed++;
      }
    }
    console.log(`[cht1] px ${part.name} ${part.box.map(Math.round).join(",")}: hiding it changes ${changed} px (seen at 20)`);
    if (changed < 20) found.out.push(`${part.name} ${part.box.map(Math.round).join(",")} is not seen (hiding it changes ${changed} px)`);
  }
  return found.out;
}

/**
 * Evidence (lead, CHT1): every chatArea slot element's box and its children
 * in DOM order, each named by what it holds (the presence row, the message
 * list, …) with its box. The presence row and the message list mount through
 * two SlotMounts, each a Portal that appends its own container to the slot.
 */
async function chatAreaOrder(): Promise<string> {
  return (await browser.execute(() => {
    const fmt = (e: Element): string => {
      const r = e.getBoundingClientRect();
      return `${Math.round(r.left)},${Math.round(r.top)} ${Math.round(r.width)}x${Math.round(r.height)}`;
    };
    const holds = (e: Element): string =>
      e.querySelector(".chat-peer-status") || e.matches(".chat-peer-status")
        ? "presence row"
        : e.querySelector(".chat-message-area") || e.matches(".chat-message-area")
          ? "message list"
          : `${e.tagName.toLowerCase()}.${(e as HTMLElement).className || ""}`;
    return JSON.stringify(
      Array.from(document.querySelectorAll<HTMLElement>('.chat-window [data-skin-role="slot:chatArea"]')).map((slot) => ({
        slot: `${slot.dataset.skinNode} ${fmt(slot)}`,
        children: Array.from(slot.children).map((c) => `${holds(c)} ${fmt(c)}`),
      })),
    );
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
        // Evidence first, so a failing check below still leaves it.
        await browser.pause(2000);
        console.log(`[slot] ${file} ${kind}: ${await slotBoxes()}`);
        const st = await stripsAndTitle();
        console.log(`[strips] ${file} ${kind}: ${st.strips}`);
        console.log(`[title] ${file} ${kind}: ${st.title}`);
        console.log(`[voice] ${file} ${kind}: ${await voiceEvidence()}`);
        await shot(`${kind}-before-checks`);
        if (kind === "dm") await idleDmOffersCall(file);
        await treeLaid(handle, title, text);
        const def = await sizeTo(CHAT_DM.def.width, CHAT_DM.def.height);
        await treeLaid(handle, title, text);
        await shot(`${kind}-default-${def}`);
        if (kind === "dm") console.log(`[order] ${file} dm: ${await chatAreaOrder()}`);
        const frame = await chatFrameChecks(kind, text ?? "");
        console.log(`[cht1] ${file} ${kind}: ${frame.length === 0 ? "title in its strip and seen; presence row, message and composer seen; no clipped art" : JSON.stringify(frame)}`);
        expect({ pack: file, window: kind, problems: frame }).toEqual({ pack: file, window: kind, problems: [] });
        const min = await sizeTo(CHAT_DM.min.width, CHAT_DM.min.height);
        await treeLaid(handle, title, text);
        await shot(`${kind}-min-${min}`);
        expect(min).not.toBe("");
      }
    });
  }

  // Lead (inactive close): a skin's close button closes every window it
  // frames. The Host answers its press with ShellOp::Close, which a chat
  // window had no runner for. Each window is reopened under its label (a
  // window keeps its label across a close).
  for (const file of ["symbiosis", "alloy"]) {
    it(`${file}: the skin's close button closes the 1:1 and group windows`, async () => {
      const pack = builtinPacks().find((p) => p.file === file);
      if (!pack) throw new Error(`no built-in pack ${file}`);
      await switchTo(browser, SETTINGS);
      const href = await applySkin(browser, me, pack.name);
      for (const [handle, kind] of [[dm, "dm"], [group, "group"]] as const) {
        await switchTo(browser, handle, NET_WAIT_MS);
        await skinShown(browser, href);
        const state = (await browser.execute(() => {
          const n = document.querySelector<HTMLElement>('.skin-tree [data-skin-role="windowButton:close"]');
          return n ? { state: n.dataset.state ?? "", cause: n.dataset.stateCause ?? "", ariaDisabled: n.getAttribute("aria-disabled") } : null;
        })) as Record<string, unknown> | null;
        console.log(`[close] ${file} ${kind}: close button ${JSON.stringify(state)}`);
        await shot(`${kind}-before-close`);
        await press(browser, await browser.$('.skin-tree [data-skin-role="windowButton:close"]'));
        await waitUntil(async () => !(await browser.getWindowHandles()).includes(handle), NET_WAIT_MS, `${file}: the ${kind} window's close button did not close it`);
        console.log(`[close] ${file} ${kind}: closed`);
        await switchTo(browser, BUDDY);
        await friendsView(browser);
        await doubleClick(browser, kind === "dm" ? await friendRow(browser, "Bea") : await byText(browser, ".group-dm-row", GROUP.slice(0, 24), NET_WAIT_MS));
        await switchTo(browser, handle, NET_WAIT_MS);
      }
      await switchTo(browser, BUDDY);
      await openSettings(browser, "Appearance");
    });
  }
});
