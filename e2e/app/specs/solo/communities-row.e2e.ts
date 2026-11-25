// Plan CB1: the Communities view's own button row, composed per pack from a
// sibling row's art (Open / Leave / New). For every pack, in the buddy list's
// Communities view: with nothing selected Open and Leave are drawn disabled
// and New is not; selecting the community row enables Open and Leave; Open
// opens that community's window, Leave raises the Host's leave intent
// (cancelled here), and New opens the Create-or-Join chooser. Each state a shot.

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, press, shown, switchTo, waitForHandle, waitUntil } from "../../../walk/helpers.ts";
import { BUDDY, closeModal, openSettings } from "../../../walk/nav.ts";
import { createCommunity, createIdentity, newUser, type User } from "../../journeys.ts";
import { applySkin, showSkinView, skinShown } from "../../journeys_skins.ts";
import { shot } from "../../shots.ts";
import { builtinPacks } from "../../skin-pack.ts";

const PACKS = ["combat", "alloy", "onyx", "xslimfire", "simple-x", "symbiosis"];
const COMMUNITY = `Row Hall ${Date.now() % 100000}`;
const NODES = { open: "CommunitiesOpen", leave: "CommunitiesLeave", new: "CommunitiesNew" } as const;

interface NodeState {
  shown: boolean;
  disabled: boolean;
}

/** Tree node `id` in the current window: drawn (not hidden, with a box) and whether it is disabled. */
async function node(id: string): Promise<NodeState> {
  return (await browser.execute((nid: string) => {
    const n = Array.from(document.querySelectorAll<HTMLElement>(`[data-skin-node="${nid}"]`)).find((e) => {
      if (e.closest("[hidden]")) return false;
      const r = e.getBoundingClientRect();
      return r.width > 0 && r.height > 0;
    });
    return n ? { shown: true, disabled: n.getAttribute("aria-disabled") === "true" } : { shown: false, disabled: false };
  }, id)) as NodeState;
}

async function pressNode(id: string): Promise<void> {
  const handle = (await browser.execute((nid: string) => {
    const n = Array.from(document.querySelectorAll<HTMLElement>(`[data-skin-node="${nid}"]`)).find((e) => !e.closest("[hidden]") && e.getBoundingClientRect().width > 0);
    if (!n) return false;
    n.setAttribute("data-press-target", "1");
    return true;
  }, id)) as boolean;
  if (!handle) throw new Error(`no drawn ${id}`);
  await press(browser, await browser.$('[data-press-target="1"]'));
  await browser.execute(() => document.querySelector('[data-press-target="1"]')?.removeAttribute("data-press-target"));
}

describe("the Communities view's row: Open / Leave / New in every pack (CB1)", function () {
  this.timeout(2_400_000);
  const me: User = { ...newUser("cbrow"), displayName: "Ari" };

  before(async () => {
    await createIdentity(browser, me);
    await createCommunity(browser, me, COMMUNITY);
  });

  for (const file of PACKS) {
    it(`${file}: disabled until a community is selected, then Open, Leave and New act`, async () => {
      const pack = builtinPacks().find((p) => p.file === file);
      if (!pack) throw new Error(`no built-in pack ${file}`);
      await switchTo(browser, BUDDY);
      await openSettings(browser, "Appearance");
      const href = await applySkin(browser, me, pack.name);
      await switchTo(browser, BUDDY);
      await skinShown(browser, href);
      // Lead (CB2): does the Servers view's own row show on its page (the
      // same slot-wrapper shape hid the Communities row)? Logged and shot.
      await showSkinView(browser, "servers");
      await browser.pause(1000);
      const servers = (await browser.execute(() => {
        const drawn = (e: Element): boolean => !e.closest("[hidden]") && e.getBoundingClientRect().width > 0;
        const row = Array.from(document.querySelectorAll('[data-skin-role="buttonRow:servers"]')).find(drawn);
        const buttons = row ? Array.from(row.querySelectorAll('[data-skin-role="button"]')).filter(drawn).map((b) => (b as HTMLElement).dataset.skinNode ?? "") : [];
        return { row: !!row, buttons };
      })) as { row: boolean; buttons: string[] };
      console.log(`[cb] ${file} servers row: ${JSON.stringify(servers)}`);
      await shot("servers-row");

      // The tree's Communities view, by its tab or a knob's stop (Combat):
      // showSkinView turns knobs; view keys are lowercase.
      await showSkinView(browser, "communities");
      await waitUntil(async () => (await node(NODES.open)).shown, NET_WAIT_MS, `${file}: the Communities row's Open is not drawn`);

      // 1. Nothing selected: Open and Leave drawn disabled (never-hide: drawn,
      // and visibly not usable), New usable. Read once the window's signals
      // are live (the tree draws before them, every node in its normal state),
      // and allow the state triggers a moment to run.
      await waitUntil(
        async () => (await browser.$('.skin-tree[data-skin-signals="live"]').isExisting().catch(() => false)),
        NET_WAIT_MS,
        `${file}: the window's skin signals never went live`,
      );
      let before = { open: await node(NODES.open), leave: await node(NODES.leave), new: await node(NODES.new) };
      await waitUntil(
        async () => {
          before = { open: await node(NODES.open), leave: await node(NODES.leave), new: await node(NODES.new) };
          return before.open.disabled && before.leave.disabled;
        },
        10_000,
        "",
      ).catch(() => undefined);
      console.log(`[cb] ${file} nothing selected (signals live): ${JSON.stringify(before)}`);
      await shot("nothing-selected");
      expect(before.open.shown && before.leave.shown && before.new.shown).toBe(true);
      expect(before.open.disabled).toBe(true);
      expect(before.leave.disabled).toBe(true);
      expect(before.new.disabled).toBe(false);

      // 2. The community row selected: Open and Leave enabled.
      await press(browser, await shown(browser, `.community-item*=${COMMUNITY.slice(0, 24)}`, NET_WAIT_MS));
      await waitUntil(
        async () => !(await node(NODES.open)).disabled && !(await node(NODES.leave)).disabled,
        NET_WAIT_MS,
        `${file}: Open and Leave stayed disabled with the community selected`,
      );
      await shot("selected");

      // 3. Open: that community's window.
      const handles = await browser.getWindowHandles();
      await pressNode(NODES.open);
      const win = await waitForHandle(browser, (h) => h.startsWith("community-") && !handles.includes(h), NET_WAIT_MS);
      await browser.switchToWindow(win);
      await shown(browser, ".community-sidebar", NET_WAIT_MS);
      await shot("open");
      await browser.closeWindow();
      await switchTo(browser, BUDDY);

      // 4. Leave: the Host's leave intent asks first (cancelled here).
      await pressNode(NODES.leave);
      await shown(browser, ".intent-dialog", NET_WAIT_MS);
      await shot("leave-intent");
      await press(browser, await shown(browser, ".intent-dialog .intent-dialog-actions button.sec-btn-secondary"));
      await waitUntil(async () => !(await browser.$(".intent-dialog").isDisplayed().catch(() => false)), NET_WAIT_MS, `${file}: the leave intent did not close on Cancel`);

      // 5. New: the Create-or-Join chooser.
      await pressNode(NODES.new);
      await waitUntil(
        async () => (await (await browser.$(".modal-title")).getText().catch(() => "")) === "New community",
        NET_WAIT_MS,
        `${file}: New opened no chooser`,
      );
      expect(await (await browser.$(".community-start-choices")).isDisplayed()).toBe(true);
      await shot("new-chooser");
      await closeModal(browser);
    });
  }
});
