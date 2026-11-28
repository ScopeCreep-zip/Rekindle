// Plan E7.14 S5 (moved from E7.7.5's e2e/skin-editor.spec.ts, which mocked
// the Host): the skin editor on the real app and the Host's own report
// (`validate_skin_draft`, rekindle_skin::draft). Settings → Appearance opens
// it as its own window; a draft started from the shown skin asks for its
// author (the field is required, not a failing check) and credits the
// source's authors after them; the licence reads as an SPDX expression or
// a licence text; a pair that fails disables Export and names its ratio,
// and the Host's suggestion, clicked, fixes it; live apply restyles the
// editor window only, never the device skin in the other windows.
//
// Export and region import go through the OS save and open dialogs, which
// WebDriver cannot drive; what they write and refuse is the Host's, tested
// in rekindle-node host/skin_draft_tests.rs (the export a byte-identical
// import, the region refused by its nine-slice, the schema export).

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, byText, press, shown, switchTo, waitForHandle, waitUntil } from "../../../walk/helpers.ts";
import { BUDDY, SETTINGS, openSettings, setValue } from "../../../walk/nav.ts";
import { createIdentity, newUser, type User } from "../../journeys.ts";

/** rekindle_skin::draft::NO_AUTHOR. */
const NO_AUTHOR = "Add your name as the author: the export credits you first.";
const AUTHOR = ".skin-editor-meta input[required]";
const LICENCE = ".skin-editor-licence";
const STATUS = ".skin-editor-status";

/** The editor's status line. */
async function status(): Promise<string> {
  return (await (await browser.$(STATUS)).getText()).trim();
}

/** Wait until the status line reads `want` (a string, exactly, or a pattern). */
async function statusIs(want: string | RegExp): Promise<void> {
  let last = "(never read)";
  try {
    await waitUntil(
      async () => {
        // A failed read is kept too: run 4 failed with every read throwing and no reason shown.
        last = await status().catch((err: unknown) => `read failed: ${err instanceof Error ? err.message : String(err)}`);
        return typeof want === "string" ? last === want : want.test(last);
      },
      NET_WAIT_MS,
      "",
    );
  } catch {
    // The status read last, once the wait is over (a message built before
    // the wait always said "(never read)", r24 on Linux and Mac), and the
    // editor's error when a check threw (the report then stays the last one).
    const shown = (await browser.execute(() => document.querySelector(".skin-editor .skin-error")?.textContent?.trim() ?? "")) as string;
    throw new Error(`the status never read ${String(want)} (last: ${JSON.stringify(last)}${shown ? `; editor error: ${JSON.stringify(shown)}` : ""})`);
  }
}

/** Whether the bar's Export… button (not Export JSON Schema…) is enabled. */
async function exportEnabled(): Promise<boolean> {
  return (await browser.execute(() => {
    const b = Array.from(document.querySelectorAll<HTMLButtonElement>(".skin-editor-bar button")).find((x) => (x.textContent ?? "").trim() === "Export…");
    if (!b) throw new Error("no Export… button");
    return !b.disabled;
  })) as boolean;
}

/** The names the credits list shows, in order. */
async function credits(): Promise<string[]> {
  return (await browser.execute(() =>
    Array.from(document.querySelectorAll(".skin-editor-credits li")).map((li) => (li.textContent ?? "").trim()),
  )) as string[];
}

/** `css`'s colour as this window resolves it, through a probe's computed `color`. */
async function tokenColour(css: string): Promise<string> {
  return (await browser.execute((v: string) => {
    const probe = document.createElement("span");
    probe.style.color = `var(${v})`;
    document.body.append(probe);
    const c = getComputedStyle(probe).color;
    probe.remove();
    return c;
  }, css)) as string;
}

/** The colour picker for token `id`. */
const picker = (id: string): string => `input.skin-token-picker[aria-label="${id}"]`;

/**
 * Start a fresh draft from the shown skin, and wait for its report: a fresh
 * draft has no author, so its status asks for one. Any status would also
 * match the last draft's, and that draft's late report then overwrote the
 * next edit (run 3: the live-apply colour never landed).
 */
async function restart(): Promise<void> {
  // A state the fresh draft undoes, so its waits read the restart's report:
  // an editor just opened already reads "add your name" with no author, and
  // both waits passed on it; "Bea" then went into the old draft and the
  // restart's authorless draft landed after it (c5802aa26, load ~30).
  // Any report but the authorless one (a draft left failing reads its count).
  await setValue(browser, AUTHOR, "restart");
  await statusIs(/^(?!Every check passes; add your name)/);
  await press(browser, await byText(browser, ".skin-editor-bar button", "Start from the current skin"));
  await statusIs("Every check passes; add your name as the author to export.");
  await waitUntil(async () => (await (await browser.$(AUTHOR)).getValue()) === "", NET_WAIT_MS, "the fresh draft kept the last author");
}

describe("the skin editor, on the Host's own report", function () {
  this.timeout(600_000);
  const me: User = newUser("skineditor");
  let editor = "";

  before(async () => {
    await createIdentity(browser, me);
  });

  it("Settings → Appearance opens the editor as its own window, never in place", async () => {
    await openSettings(browser, "Appearance");
    const settings = await browser.getWindowHandle();
    const before = await browser.getWindowHandles();
    await press(browser, await byText(browser, "button", "Edit a skin…"));
    editor = await waitForHandle(browser, (h) => h === "skin-editor" && !before.includes(h), NET_WAIT_MS);
    expect(await browser.execute(() => document.querySelectorAll(".skin-editor, #skin-draft-tokens").length)).toBe(0);
    expect(settings).not.toBe(editor);
    await browser.switchToWindow(editor);
    await shown(browser, ".skin-editor", NET_WAIT_MS);
    await statusIs(/^Every check passes/);
  });

  it("a started draft asks for its author, then credits them first, before the source's authors", async () => {
    await browser.switchToWindow(editor);
    await restart();
    await statusIs("Every check passes; add your name as the author to export.");
    expect(await exportEnabled()).toBe(false);
    const author = await browser.$(AUTHOR);
    expect(await author.getAttribute("aria-invalid")).toBe("true");
    expect((await (await browser.$("#skin-editor-author-hint")).getText()).trim()).toBe(NO_AUTHOR);
    const source = await credits();
    expect(source[0]).toBe("(no author yet)");

    await setValue(browser, AUTHOR, "Bea");
    await statusIs("Every check passes.");
    expect(await exportEnabled()).toBe(true);
    expect(await author.getAttribute("aria-invalid")).toBe("false");
    expect(await credits()).toEqual(["Bea", ...source.slice(1)]);
  });

  it("the licence reads as an SPDX expression or as a licence text", async () => {
    await browser.switchToWindow(editor);
    const kind = async () => (await (await browser.$("#skin-editor-licence-kind")).getText()).trim();
    await setValue(browser, LICENCE, "MIT");
    await waitUntil(async () => (await kind()) === "Read as an SPDX expression.", NET_WAIT_MS, `MIT did not read as SPDX (${await kind()})`);
    await setValue(browser, LICENCE, "Free for any use, with credit.");
    await waitUntil(async () => (await kind()) === "Read as a licence text.", NET_WAIT_MS, `the text did not read as a licence text (${await kind()})`);
  });

  it("a failing pair disables Export and names its ratio; the Host's suggestions, clicked, fix it", async () => {
    await browser.switchToWindow(editor);
    await setValue(browser, AUTHOR, "Bea");
    await statusIs("Every check passes.");
    // A declared pair whose text and ground are both pickers: the text set to the ground's own colour reads 1.00:1.
    const pair = (await browser.execute((sel: string) => {
      for (const li of Array.from(document.querySelectorAll(".skin-pair-name"))) {
        const m = /^(\S+) on (\S+) /.exec((li.textContent ?? "").trim());
        if (!m) continue;
        const fg = document.querySelector<HTMLInputElement>(sel.replace("ID", m[1]));
        const ground = document.querySelector<HTMLInputElement>(sel.replace("ID", m[2]));
        if (fg && ground) return { fg: m[1], ground: m[2], colour: ground.value };
      }
      return null;
    }, picker("ID"))) as { fg: string; ground: string; colour: string } | null;
    if (!pair) throw new Error("no declared pair has pickers for both its text and its ground");
    await setValue(browser, picker(pair.fg), pair.colour);
    await statusIs(/^\d+ checks? fails?; export is disabled\.$/);
    expect(await exportEnabled()).toBe(false);
    const failing = (await browser.execute(() =>
      Array.from(document.querySelectorAll(".skin-pair-fail")).map((li) => (li.textContent ?? "").replace(/\s+/g, " ").trim()),
    )) as string[];
    const named = failing.find((t) => t.startsWith(`${pair.fg} on ${pair.ground} `));
    expect(named).toMatch(/1\.00:1 \/ \d\.\d\d:1 Fail/);

    // Each click is one suggestion, applied only by the author's click; the
    // token's other pairs may need their own, so at most one per failing pair.
    for (let i = 0; i < 12 && (await browser.$$(".skin-pair-fail .skin-suggest")).length > 0; i++) {
      const before = await status();
      // The first failing pair's suggestion (one per failing pair; webdriverio's `$` refuses several).
      await press(browser, (await browser.$$(".skin-pair-fail .skin-suggest"))[0]);
      await waitUntil(async () => (await status()) !== before || (await browser.$$(".skin-pair-fail")).length === 0, NET_WAIT_MS, "a suggestion changed nothing");
    }
    expect((await browser.$$(".skin-pair-fail")).length).toBe(0);
    await statusIs("Every check passes.");
    expect(await exportEnabled()).toBe(true);
    expect(await (await browser.$(picker(pair.fg))).getValue()).not.toBe(pair.colour);
  });

  it("live apply restyles the editor window, never the device skin in Settings or the buddy list", async () => {
    const css = "--color-xfire-accent-text";
    const orange = "rgb(255, 136, 0)";
    await switchTo(browser, BUDDY);
    const buddyBefore = await tokenColour(css);
    expect(buddyBefore).not.toBe(orange);
    await browser.switchToWindow(editor);
    await restart();
    await setValue(browser, picker("accent.text"), "#ff8800");
    await waitUntil(async () => (await tokenColour(css)) === orange, NET_WAIT_MS, `the editor's ${css} never became ${orange}`);
    await switchTo(browser, BUDDY);
    expect(await tokenColour(css)).toBe(buddyBefore);
    await switchTo(browser, SETTINGS);
    expect(await tokenColour(css)).toBe(buddyBefore);
  });
});
