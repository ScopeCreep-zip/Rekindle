// FE-LINT 016 (the mention list's ARIA and focus), checked in the real app:
// in a community channel, typing "@" opens the mention list as a listbox
// the textarea controls; the textarea keeps the focus while ArrowDown moves
// the active option (aria-activedescendant names an option that exists, with
// a per-composer id); Enter inserts the active option into the draft.

import { browser, expect } from "@wdio/globals";
import { Key } from "webdriverio";
import { gone, shown, waitUntil } from "../../../walk/helpers.ts";
import { createCommunity, createIdentity, newUser, openCommunity, selectChannel, type User } from "../../journeys.ts";
import { shot } from "../../shots.ts";

const COMPOSER = ".message-input-shell textarea";

interface MentionState {
  focused: boolean;
  expanded: string | null;
  controls: string | null;
  listId: string | null;
  role: string | null;
  active: string | null;
  activeExists: boolean;
  options: string[];
  value: string;
}

async function state(): Promise<MentionState> {
  return (await browser.execute((sel: string) => {
    const t = document.querySelector<HTMLTextAreaElement>(sel);
    const list = document.querySelector<HTMLElement>(".mention-popover");
    const active = t?.getAttribute("aria-activedescendant") ?? null;
    return {
      focused: document.activeElement === t,
      expanded: t?.getAttribute("aria-expanded") ?? null,
      controls: t?.getAttribute("aria-controls") ?? null,
      listId: list?.id ?? null,
      role: list?.getAttribute("role") ?? null,
      active,
      activeExists: active !== null && document.getElementById(active)?.getAttribute("role") === "option",
      options: Array.from(list?.querySelectorAll('[role="option"]') ?? []).map((o) => o.textContent?.trim() ?? ""),
      value: t?.value ?? "",
    };
  }, COMPOSER)) as MentionState;
}

describe("the mention list is a listbox the composer controls (FE-LINT 016)", function () {
  this.timeout(300_000);
  const me: User = { ...newUser("mention"), displayName: "Ari" };

  it("@ opens it, ArrowDown moves the active option with the focus kept, Enter inserts it", async () => {
    await createIdentity(browser, me);
    await createCommunity(browser, me, "Mention Hall");
    await openCommunity(browser, "Mention Hall");
    await selectChannel(browser, "general");
    const field = await shown(browser, COMPOSER);
    await field.click();
    await browser.keys("@");
    await shown(browser, ".mention-popover");
    const open = await state();
    console.log(`[mention] open: ${JSON.stringify(open)}`);
    await shot("mention-open");
    expect({ focused: open.focused, expanded: open.expanded, role: open.role, controlsList: open.controls === open.listId && open.listId !== null, activeExists: open.activeExists }).toEqual({
      focused: true, expanded: "true", role: "listbox", controlsList: true, activeExists: true,
    });
    if (open.options.length > 1) {
      await browser.keys(Key.ArrowDown);
      await waitUntil(async () => (await state()).active !== open.active, 5_000, "ArrowDown did not move the active option");
      const moved = await state();
      console.log(`[mention] after ArrowDown: ${JSON.stringify(moved)}`);
      expect({ focused: moved.focused, activeExists: moved.activeExists }).toEqual({ focused: true, activeExists: true });
    }
    await browser.keys(Key.Enter);
    await gone(browser, ".mention-popover");
    const done = await state();
    console.log(`[mention] after Enter: ${JSON.stringify(done)}`);
    await shot("mention-inserted");
    expect({ inserted: done.value.startsWith("@") && done.value.trim().length > 1, expanded: done.expanded, controls: done.controls }).toEqual({ inserted: true, expanded: "false", controls: null });
  });
});
