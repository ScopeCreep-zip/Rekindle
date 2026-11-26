// Plan CR1: a disabled node says why, never a reason-less one. In each pack's
// buddy list with nothing selected (so selection-dependent controls are
// disabled), every drawn node the tree marks disabled (`aria-disabled`)
// carries the Host's reason as its `aria-description` and `title`; one the
// pack draws no disabled art for (`data-disabled-by="shell"`) is dimmed, and
// one with its own art (`pack`) is not. Each node logged as `[cr1]` and shot.
//
// Lead ruling (CR1): a pack's unfocused-window look (`deactivate` →
// `SetState(Disabled)`) is not a refusal. Such a node shows its Disabled art
// with cause `windowInactive` (`data-state-cause`), stays pressable and is
// never `aria-disabled`; the checks go by cause, never by node name. And an
// inactive window's close button still closes it (Symbiosis's community
// window, which wears the pack's chat frame and follows its signals, SKW1).
//
// Lead (owner's rule: a disabled control is *visibly* disabled): every
// refused node looks different from its enabled self. Its box is shot as
// served, then as its enabled self (`data-state="normal"`, no
// `aria-disabled` or `data-disabled-by`), and at least 20 pixels must differ
// (Combat's round Open/Leave drew the same art in both).

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, press, switchTo, waitUntil } from "../../../walk/helpers.ts";
import { BUDDY, SETTINGS, closeSettings, openSettings } from "../../../walk/nav.ts";
import { createCommunity, createIdentity, newUser, openCommunity, type User } from "../../journeys.ts";
import { applySkin, skinShown } from "../../journeys_skins.ts";
import { artContribution, disabledLookWithBoxes, ownArt } from "../../disabled-look.ts";
import { shot } from "../../shots.ts";
import { builtinPacks } from "../../skin-pack.ts";

const PACKS = ["symbiosis", "onyx", "combat", "alloy", "xslimfire", "simple-x"];

interface Disabled {
  node: string;
  reason: string;
  title: string;
  by: string;
  opacity: string;
  cause: string;
}

/** The current window's drawn nodes in their Disabled art for `windowInactive`: `aria-disabled` must be absent. */
async function inactiveNodes(): Promise<{ node: string; ariaDisabled: string | null }[]> {
  return (await browser.execute(() =>
    Array.from(document.querySelectorAll<HTMLElement>('[data-skin-node][data-state-cause="windowInactive"]'))
      .filter((n) => !n.closest("[hidden]") && n.getBoundingClientRect().width > 0)
      .map((n) => ({ node: n.dataset.skinNode ?? "", ariaDisabled: n.getAttribute("aria-disabled") })),
  )) as { node: string; ariaDisabled: string | null }[];
}

describe("a disabled node says why (CR1)", function () {
  this.timeout(1_200_000);
  const me: User = { ...newUser("cr1"), displayName: "Ari" };

  before(async () => {
    await createIdentity(browser, me);
  });

  for (const file of PACKS) {
    it(`${file}: every disabled node carries the Host's reason; only shell-drawn ones are dimmed`, async () => {
      const pack = builtinPacks().find((p) => p.file === file);
      if (!pack) throw new Error(`no built-in pack ${file}`);
      await switchTo(browser, BUDDY);
      await openSettings(browser, "Appearance");
      const href = await applySkin(browser, me, pack.name);
      await switchTo(browser, BUDDY);
      await skinShown(browser, href);
      await browser.pause(2000);
      // Disabled states are pack triggers fired from the Host's signal set
      // (signalEffects), so they exist only once the window's signals are
      // live; the state is logged either way.
      const state = async (): Promise<string> =>
        (await browser.execute(() => {
          const t = document.querySelector<HTMLElement>(".skin-tree");
          return JSON.stringify({
            signals: t?.dataset.skinSignals ?? "none",
            cannot: t?.dataset.skinCannot ?? "",
            ariaDisabled: document.querySelectorAll('[data-skin-node][aria-disabled="true"]').length,
          });
        })) as string;
      await waitUntil(async () => (await browser.$('.skin-tree[data-skin-signals="live"]').isExisting().catch(() => false)), NET_WAIT_MS, "").catch(async () => {
        throw new Error(`${file}: the window's skin signals never went live (${await state()})`);
      });
      console.log(`[cr1] ${file} signals: ${await state()}`);
      const nodes = (await browser.execute(() =>
        Array.from(document.querySelectorAll<HTMLElement>('[data-skin-node][aria-disabled="true"]'))
          .filter((n) => !n.closest("[hidden]") && n.getBoundingClientRect().width > 0)
          .map((n) => ({
            node: n.dataset.skinNode ?? "",
            reason: n.getAttribute("aria-description") ?? "",
            title: n.getAttribute("title") ?? "",
            by: n.dataset.disabledBy ?? "",
            opacity: getComputedStyle(n).opacity,
            cause: n.dataset.stateCause ?? "",
          })),
      )) as Disabled[];
      for (const d of nodes) console.log(`[cr1] ${file} ${d.node}: cause=${d.cause || "?"} by=${d.by || "?"} opacity=${d.opacity} reason="${d.reason}"`);
      const measured = await disabledLookWithBoxes(nodes.map((d) => d.node));
      const look = Object.fromEntries(Object.entries(measured).map(([id, v]) => [id, v.changed]));
      console.log(`[cr1] ${file} measured boxes: ${JSON.stringify(measured)}`);
      // A refused node that changes nothing: what is on top at its centre.
      for (const [id, v] of Object.entries(measured).filter(([, m]) => m.changed < 20)) {
        const top = (await browser.execute((box: number[]) => {
          const t = document.elementFromPoint(box[0] + box[2] / 2, box[1] + box[3] / 2) as HTMLElement | null;
          const node = t?.closest<HTMLElement>("[data-skin-node]");
          return `top=${t?.tagName.toLowerCase()}.${t?.className ?? ""} in node ${node?.dataset.skinNode ?? "none"} (${node?.dataset.skinRole ?? "-"}) art=${node ? getComputedStyle(node).backgroundImage.slice(0, 90) : "-"}`;
        }, v.box)) as string;
        console.log(`[cr1] ${file} ${id} changes ${v.changed} px; at its centre: ${top}`);
        // Lead: the node's own art's alpha at its box centre (the served
        // image drawn at the node's background size and position), and every
        // node with art under that point (which one draws the button seen).
        const art = `${await ownArt(id)} | contribution: ${await artContribution(id)}`;
        console.log(`[cr1] ${file} ${id} own art: ${art}`);
      }
      console.log(`[cr1] ${file} disabled vs enabled self, pixels changed: ${JSON.stringify(look)} (visibly disabled at 20)`);
      const inactive = await inactiveNodes();
      console.log(`[cr1] ${file} windowInactive (pressable, not aria-disabled): ${JSON.stringify(inactive)}`);
      await shot(`${file}-disabled`);
      expect(nodes.length).toBeGreaterThan(0);
      expect(nodes.filter((d) => d.reason.trim() === "").map((d) => d.node)).toEqual([]);
      expect(nodes.filter((d) => d.title !== d.reason).map((d) => d.node)).toEqual([]);
      expect(nodes.filter((d) => d.by === "shell" && d.opacity !== "0.5").map((d) => d.node)).toEqual([]);
      expect(nodes.filter((d) => d.by === "pack" && d.opacity !== "1").map((d) => d.node)).toEqual([]);
      // By cause: every refused node names a refusal cause and who draws it.
      expect(nodes.filter((d) => d.cause === "" || d.cause === "windowInactive" || d.by === "").map((d) => d.node)).toEqual([]);
      expect(inactive.filter((i) => i.ariaDisabled !== null).map((i) => i.node)).toEqual([]);
      // Visibly disabled: each refused node differs from its enabled self.
      expect(Object.entries(look).filter(([, n]) => n < 20).map(([id]) => id)).toEqual([]);
    });
  }

  it("symbiosis: an inactive community window's close button, in the pack's Disabled art, still closes it", async () => {
    const pack = builtinPacks().find((p) => p.file === "symbiosis");
    if (!pack) throw new Error("no built-in pack symbiosis");
    await switchTo(browser, BUDDY);
    await openSettings(browser, "Appearance");
    const href = await applySkin(browser, me, pack.name);
    const name = `Inactive ${Date.now() % 100000}`;
    await switchTo(browser, BUDDY);
    await createCommunity(browser, me, name);
    await openCommunity(browser, name);
    const handle = await browser.getWindowHandle();
    await skinShown(browser, href);
    // Lead (Linux run): on an unlocked desktop the fresh window is focused.
    // It is unfocused through the app's own path: Settings opened from the
    // buddy list's menu comes up focused (a locked screen leaves every
    // window unfocused already). A focused window is never pressed and
    // called inactive: the wait below fails loudly instead.
    await openSettings(browser, "Appearance");
    await browser.switchToWindow(handle);
    // The window follows its signals (SKW1); unfocused, the pack's
    // deactivate trigger draws its close button in the Disabled art.
    let close: { node: string; cause: string; ariaDisabled: string | null; role: string } | null = null;
    await waitUntil(
      async () => {
        close = (await browser.execute(() => {
          const n = document.querySelector<HTMLElement>('.skin-tree [data-skin-role="windowButton:close"]');
          return n ? { node: n.dataset.skinNode ?? "", cause: n.dataset.stateCause ?? "", ariaDisabled: n.getAttribute("aria-disabled"), role: n.getAttribute("role") ?? "" } : null;
        })) as typeof close;
        return close !== null && close.cause === "windowInactive";
      },
      NET_WAIT_MS,
      "",
    ).catch(() => {
      throw new Error(`the community window never deactivated (its close button is not in its windowInactive art), so no inactive press is made: ${JSON.stringify(close)}`);
    });
    console.log(`[cr1] symbiosis community close: ${JSON.stringify(close)}`);
    // What takes a press at the button's centre: the nearest node listening
    // for pointer events there (a drag strip over the button would take it).
    const atClose = (await browser.execute(() => {
      const n = document.querySelector<HTMLElement>('.skin-tree [data-skin-role="windowButton:close"]');
      if (!n) return "no close button";
      const r = n.getBoundingClientRect();
      const top = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
      const listener = top?.closest<HTMLElement>("[data-skin-on]");
      return `top=${top?.tagName.toLowerCase()}.${(top as HTMLElement | null)?.className ?? ""} listener=${listener?.dataset.skinNode ?? "none"} (${listener?.dataset.skinRole ?? "-"}, on=${listener?.dataset.skinOn ?? "-"}) is the button: ${listener === n}`;
    })) as string;
    console.log(`[cr1] symbiosis community close, at its centre: ${atClose}`);
    await shot("symbiosis-inactive-close");
    expect(close!.ariaDisabled).toBeNull();
    // Lead (inactive close): trace the press. Every toast the window shows
    // after it is collected from the page (a served operation with no
    // runner says "<op> has no handler in this window"), with the node's
    // state at the press.
    await browser.execute(() => {
      const w = window as unknown as { __closeToasts?: string[] };
      w.__closeToasts = [];
      new MutationObserver(() => {
        for (const t of Array.from(document.querySelectorAll(".toast-item"))) {
          const text = (t.textContent ?? "").trim();
          if (text && !w.__closeToasts!.includes(text)) w.__closeToasts!.push(text);
        }
      }).observe(document.body, { childList: true, subtree: true, characterData: true });
    });
    await (await browser.$('.skin-tree [data-skin-role="windowButton:close"]')).click();
    await browser.pause(3000);
    const traced = (await browser.execute(() => {
      const w = window as unknown as { __closeToasts?: string[] };
      const n = document.querySelector<HTMLElement>('.skin-tree [data-skin-role="windowButton:close"]');
      return { toasts: w.__closeToasts ?? [], action: n?.dataset.skinAction ?? "", state: n?.dataset.state ?? "", cause: n?.dataset.stateCause ?? "" };
    }).catch((e: unknown) => ({ closed: String(e).slice(0, 80) }))) as Record<string, unknown>;
    console.log(`[cr1] symbiosis inactive close, after the press: ${JSON.stringify(traced)}`);
    await browser.switchToWindow(BUDDY).catch(() => undefined);
    await waitUntil(async () => !(await browser.getWindowHandles()).includes(handle), NET_WAIT_MS, "the inactive community window's close button did not close it");
    console.log("[cr1] symbiosis community window closed by its inactive close button");
    // Settings, opened to take the focus, closes too, so the next test's
    // Settings opens fresh and frontmost.
    await switchTo(browser, SETTINGS);
    await closeSettings(browser);
  });

  // Lead: close works in every window it frames, whatever its focus (the
  // cause was a missing runner, never inactivity). Each pack's community
  // window is opened, its close pressed once, and it must close; the close
  // button's state at the press is logged.
  for (const file of ["symbiosis", "alloy"]) {
    it(`${file}: a community window's close button closes it`, async () => {
      const pack = builtinPacks().find((p) => p.file === file);
      if (!pack) throw new Error(`no built-in pack ${file}`);
      await switchTo(browser, BUDDY);
      await openSettings(browser, "Appearance");
      const href = await applySkin(browser, me, pack.name);
      const name = `Close ${file} ${Date.now() % 100000}`;
      await switchTo(browser, BUDDY);
      await createCommunity(browser, me, name);
      await openCommunity(browser, name);
      const handle = await browser.getWindowHandle();
      await skinShown(browser, href);
      const state = (await browser.execute(() => {
        const n = document.querySelector<HTMLElement>('.skin-tree [data-skin-role="windowButton:close"]');
        return n ? { state: n.dataset.state ?? "", cause: n.dataset.stateCause ?? "", ariaDisabled: n.getAttribute("aria-disabled"), focused: document.hasFocus() } : null;
      })) as Record<string, unknown> | null;
      console.log(`[close] ${file} community: close button ${JSON.stringify(state)}`);
      await shot(`${file}-community-before-close`);
      await press(browser, await browser.$('.skin-tree [data-skin-role="windowButton:close"]'));
      await browser.switchToWindow(BUDDY).catch(() => undefined);
      await waitUntil(async () => !(await browser.getWindowHandles()).includes(handle), NET_WAIT_MS, `${file}: the community window's close button did not close it`);
      console.log(`[close] ${file} community: closed`);
    });
  }
});
