// Plan ST1 (contract-st1-settings-theme.md): Settings follows the theme's
// dialog roles. For every built-in pack, each Settings element's computed
// colour is the served token the contract names for it; the six selects'
// open lists (Kobalte's listbox, DOM the skin colours where a native
// select's list is the OS's) are drawn from the tokens too; one select is
// driven by keyboard as the APG listbox pattern has it; and a dialog
// outside Settings (Add Friend) shows the shared form classes take the
// same roles. Colours are compared as the engine computes them: a token is
// read by resolving `var(--token)` on a probe element in the same window.

import { browser, expect } from "@wdio/globals";
import { Key } from "webdriverio";
import { BUDDY, SETTINGS, closeModal, openAddFriend, openSettings } from "../../../walk/nav.ts";
import { NET_WAIT_MS, byText, gone, press, shown, switchTo, waitUntil } from "../../../walk/helpers.ts";
import { createIdentity, newUser, type User } from "../../journeys.ts";
import { applySkin } from "../../journeys_skins.ts";
import { decodePng } from "../../png.ts";
import { saveImage, shot } from "../../shots.ts";
import { ACCESSIBLE_VARIANT, builtinPacks } from "../../skin-pack.ts";

const PACKS = builtinPacks().filter((p) => p.file !== ACCESSIBLE_VARIANT.file);

/** One expectation: `selector`'s computed `property` is token `token`. */
interface Want {
  what: string;
  selector: string;
  property: string;
  token: string;
}

/** Each check's computed colour and the token's, as the engine resolves them; mismatches by name. */
async function colours(wants: Want[]): Promise<{ mismatched: string[]; read: string[] }> {
  return (await browser.execute((list: Want[]) => {
    const probe = document.createElement("div");
    document.body.append(probe);
    const tokenColour = (token: string): string => {
      probe.style.color = `var(--color-xfire-${token})`;
      return getComputedStyle(probe).color;
    };
    const mismatched: string[] = [];
    const read: string[] = [];
    for (const w of list) {
      const el = Array.from(document.querySelectorAll<HTMLElement>(w.selector)).find((e) => e.getBoundingClientRect().width > 0);
      const want = tokenColour(w.token);
      const got = el ? getComputedStyle(el).getPropertyValue(w.property) : "absent";
      read.push(`${w.what}: ${got} (${w.token} ${want})`);
      if (got !== want) mismatched.push(`${w.what}: ${w.property} ${got}, want ${w.token} ${want}`);
    }
    probe.remove();
    return { mismatched, read };
  }, wants)) as { mismatched: string[]; read: string[] };
}

/** The Settings tab `label`, chosen by its rail button. */
async function tab(label: string): Promise<void> {
  await switchTo(browser, SETTINGS);
  await press(browser, await byText(browser, ".form-tab", label));
  await shown(browser, `.form-tab[aria-selected="true"]`);
}

/** Open the select named by `labelId`, check its open list's colours, close it with Escape. */
async function openList(labelId: string): Promise<string[]> {
  const trigger = `.settings-select-trigger[aria-labelledby~="${labelId}"]`;
  await press(browser, await shown(browser, trigger));
  await shown(browser, ".settings-select-content", NET_WAIT_MS);
  const { mismatched, read } = await colours([
    { what: `${labelId} list ground`, selector: ".settings-select-content", property: "background-color", token: "bg-dark" },
    ...((await browser.$$(".settings-select-item:not([data-selected]):not([data-highlighted])")).length > 0
      ? [{ what: `${labelId} row text`, selector: ".settings-select-item:not([data-selected]):not([data-highlighted])", property: "color", token: "text" }]
      : []),
    { what: `${labelId} chosen row ground`, selector: ".settings-select-item[data-selected]", property: "background-color", token: "accent" },
    { what: `${labelId} chosen row text`, selector: ".settings-select-item[data-selected]", property: "color", token: "selection-text" },
  ]);
  console.log(`[st1] ${labelId} open list: ${JSON.stringify(read)}`);
  await browser.keys(Key.Escape);
  await gone(browser, ".settings-select-content");
  return mismatched;
}

describe("Settings follows the theme's dialog roles (ST1)", function () {
  this.timeout(2_400_000);
  const me: User = { ...newUser("st1"), displayName: "Ari" };

  before(async () => {
    // Auto-away stays on: its select is one of the six.
    await createIdentity(browser, me, { keepAutoAway: true });
    await openSettings(browser, "Appearance");
  });

  for (const pack of PACKS) {
    it(`${pack.file}: every Settings role is its served token, and each select's open list too`, async () => {
      await tab("Appearance");
      await applySkin(browser, me, pack.name);
      await tab("Appearance");
      const found: string[] = [];
      const rail = await colours([
        { what: "rail ground", selector: ".form-tabs", property: "background-color", token: "menubar-bg" },
        { what: "tab label", selector: '.form-tab:not([aria-selected="true"])', property: "color", token: "menubar-text" },
        { what: "chosen tab ground", selector: '.form-tab[aria-selected="true"]', property: "background-color", token: "accent" },
        { what: "chosen tab label", selector: '.form-tab[aria-selected="true"]', property: "color", token: "selection-text" },
        { what: "section title", selector: ".settings-section-title", property: "color", token: "text-secondary" },
        { what: "chosen skin row", selector: ".skin-row-shown", property: "background-color", token: "accent" },
        { what: "chosen skin row text", selector: ".skin-row-shown", property: "color", token: "selection-text" },
        { what: "skin-row button", selector: ".skin-row-btn:not(:disabled)", property: "background-color", token: "bg-secondary" },
      ]);
      console.log(`[st1] ${pack.file} appearance: ${JSON.stringify(rail.read)}`);
      found.push(...rail.mismatched);
      await shot(`${pack.file}-settings-appearance`);

      await tab("Application");
      const app = await colours([
        { what: "field label", selector: ".form-field-label", property: "color", token: "text-status" },
        { what: "hint", selector: ".settings-hint", property: "color", token: "text-status" },
        { what: "checkbox box", selector: 'input[type="checkbox"]:not(:checked)', property: "background-color", token: "input-bg" },
        { what: "checkbox outline", selector: 'input[type="checkbox"]:not(:checked)', property: "border-top-color", token: "progress-outline" },
        { what: "checked checkbox", selector: 'input[type="checkbox"]:checked', property: "background-color", token: "accent" },
        { what: "select ground", selector: ".settings-select-trigger", property: "background-color", token: "input-bg" },
        { what: "select text", selector: ".settings-select-trigger", property: "color", token: "input-text" },
        { what: "select outline", selector: ".settings-select-trigger", property: "border-top-color", token: "progress-outline" },
      ]);
      console.log(`[st1] ${pack.file} application: ${JSON.stringify(app.read)}`);
      const chevronText = (await browser.execute(() => document.querySelector(".settings-select-chevron")?.textContent ?? "absent")) as string;
      if (chevronText !== "") found.push(`chevron holds text "${chevronText}" (the CSS draws it)`);
      found.push(...app.mismatched);
      found.push(...(await openList("settings-auto-away-label")));
      // The open list over the window, for the shot.
      await press(browser, await shown(browser, '.settings-select-trigger[aria-labelledby~="settings-auto-away-label"]'));
      await shown(browser, ".settings-select-content");
      await shot(`${pack.file}-settings-select-open`);
      await browser.keys(Key.Escape);
      await gone(browser, ".settings-select-content");

      await tab("Notifications");
      const notes = await colours([
        { what: "slider outline", selector: 'input[type="range"]', property: "border-top-color", token: "progress-outline" },
      ]);
      found.push(...notes.mismatched);
      found.push(...(await openList("settings-notification-content-label")));
      await tab("Audio");
      found.push(...(await openList("settings-input-device-label")));
      found.push(...(await openList("settings-output-device-label")));
      await tab("Video");
      found.push(...(await openList("settings-camera-label")));
      await shot(`${pack.file}-settings-video`);
      // The timezone select is disabled while quiet hours are off, so its
      // list stays shut: its closed trigger is the disabled text.
      await tab("Notifications");
      const zone = await colours([
        { what: "disabled select text", selector: '.settings-select-trigger[aria-labelledby~="settings-timezone-label"]', property: "color", token: "text-disabled" },
      ]);
      found.push(...zone.mismatched);
      console.log(`[st1] ${pack.file} mismatches: ${JSON.stringify(found)}`);
      expect({ pack: pack.file, mismatched: found }).toEqual({ pack: pack.file, mismatched: [] });
      await tab("Appearance");
    });
  }

  it("a select is driven by keyboard as the APG listbox has it (arrows, Home, End, type-ahead, Enter, Escape)", async () => {
    await tab("Application");
    const trigger = '.settings-select-trigger[aria-labelledby~="settings-auto-away-label"]';
    const text = async (): Promise<string> => (await (await browser.$(`${trigger} .settings-select-value`)).getText()).trim();
    const highlighted = async (): Promise<string> =>
      ((await browser.execute(() => document.querySelector(".settings-select-item[data-highlighted]")?.textContent?.trim() ?? "none")) as string);
    const before = await text();
    await browser.execute((sel: string) => document.querySelector<HTMLElement>(sel)?.focus(), trigger);
    await browser.keys(Key.ArrowDown);
    await shown(browser, ".settings-select-content");
    await browser.keys(Key.End);
    await waitUntil(async () => (await highlighted()) === "1 hour", 5_000, `End did not reach the last option (${await highlighted()})`);
    await browser.keys(Key.Home);
    await waitUntil(async () => (await highlighted()) === "Disabled", 5_000, `Home did not reach the first option (${await highlighted()})`);
    await browser.keys("3");
    await waitUntil(async () => (await highlighted()) === "30 minutes", 5_000, `type-ahead "3" did not reach 30 minutes (${await highlighted()})`);
    await browser.keys(Key.Enter);
    await gone(browser, ".settings-select-content");
    // Host-driven: the trigger shows the stored value once the Host has it.
    await waitUntil(async () => (await text()) === "30 minutes", NET_WAIT_MS, `the choice was not stored (${await text()})`);
    const focusBack = (await browser.execute((sel: string) => document.activeElement === document.querySelector(sel), trigger)) as boolean;
    // Escape closes without choosing, focus back on the trigger.
    await browser.keys(Key.ArrowDown);
    await shown(browser, ".settings-select-content");
    await browser.keys(Key.ArrowDown);
    await browser.keys(Key.Escape);
    await gone(browser, ".settings-select-content");
    const afterEscape = await text();
    const focusAfterEscape = (await browser.execute((sel: string) => document.activeElement === document.querySelector(sel), trigger)) as boolean;
    console.log(`[st1] keyboard: before "${before}", chosen "30 minutes", after Escape "${afterEscape}", focus back ${focusBack}/${focusAfterEscape}`);
    expect({ afterEscape, focusBack, focusAfterEscape }).toEqual({ afterEscape: "30 minutes", focusBack: true, focusAfterEscape: true });
  });

  /**
   * The selection highlight as painted, not as computed: the most common
   * colour inside the field's content box that is not the field's own
   * ground (glyph pixels are a minority of the highlighted run). WebKit
   * transforms an author ::selection colour as it paints
   * (RenderThemeMac::transformSelectionBackgroundColor), so only the pixels
   * say what the user sees. The crop is saved beside the shot.
   */
  const paintedSelection = async (sel: string, name: string): Promise<{ painted: string; ground: string }> => {
    const box = (await browser.execute((s: string) => {
      const f = document.querySelector<HTMLInputElement>(s);
      if (!f) return null;
      const r = f.getBoundingClientRect();
      const cs = getComputedStyle(f);
      const pad = (k: string): number => Number.parseFloat(cs.getPropertyValue(k)) || 0;
      return {
        x: r.left + pad("padding-left") + pad("border-left-width"),
        y: r.top + pad("padding-top") + pad("border-top-width"),
        w: r.width - pad("padding-left") - pad("padding-right") - pad("border-left-width") - pad("border-right-width"),
        h: r.height - pad("padding-top") - pad("padding-bottom") - pad("border-top-width") - pad("border-bottom-width"),
        vw: window.innerWidth,
      };
    }, sel)) as { x: number; y: number; w: number; h: number; vw: number } | null;
    if (!box) throw new Error(`no field ${sel}`);
    const img = decodePng(Buffer.from(await browser.takeScreenshot(), "base64"));
    const k = img.width / box.vw;
    const x0 = Math.round(box.x * k), y0 = Math.round(box.y * k);
    const w = Math.max(1, Math.round(box.w * k)), h = Math.max(1, Math.round(box.h * k));
    const crop = { width: w, height: h, data: new Uint8Array(w * h * 4) };
    const counts = new Map<string, number>();
    for (let y = 0; y < h; y++) {
      for (let x = 0; x < w; x++) {
        const i = ((y0 + y) * img.width + (x0 + x)) * 4;
        const o = (y * w + x) * 4;
        for (let c = 0; c < 4; c++) crop.data[o + c] = img.data[i + c];
        const key = `rgb(${img.data[i]}, ${img.data[i + 1]}, ${img.data[i + 2]})`;
        counts.set(key, (counts.get(key) ?? 0) + 1);
      }
    }
    saveImage(name, crop);
    const ranked = [...counts.entries()].sort((a, b) => b[1] - a[1]).map(([c]) => c);
    const ground = (await browser.execute((s: string) => {
      const f = document.querySelector<HTMLInputElement>(s);
      return f ? getComputedStyle(f).backgroundColor : "";
    }, sel)) as string;
    return { painted: ranked.find((c) => c !== ground) ?? "", ground };
  };

  it("selected text is the theme's highlight, focused and unfocused (ST1)", async () => {
    await tab("Profile");
    const field = '.settings-content input.form-input[type="text"]';
    await shown(browser, field, NET_WAIT_MS);
    // Select the field's whole text right before each sample, and say what
    // holds: with the OS window unfocused the field can lose its focus and
    // selection between samples, and a shot with no selection is no answer
    // about the highlight's colour (the Mac's run at 14c97d213).
    const selectAll = async (when: string): Promise<void> => {
      const held = (await browser.execute((sel: string) => {
        const f = document.querySelector<HTMLInputElement>(sel);
        f?.focus();
        f?.select();
        return { focused: document.activeElement === f, start: f?.selectionStart ?? -1, end: f?.selectionEnd ?? -1, length: f?.value.length ?? -1 };
      }, field)) as { focused: boolean; start: number; end: number; length: number };
      console.log(`[st1] selection ${when}: ${JSON.stringify(held)}`);
      expect({ focused: held.focused, whole: held.start === 0 && held.end === held.length && held.length > 0 }).toEqual({ focused: true, whole: true });
      // The paint settles before the capture, as selection-paint-probe waits.
      await browser.pause(300);
    };
    const read = async (): Promise<{ focus: string; bg: string; fg: string; want: Record<string, string> }> =>
      (await browser.execute((sel: string) => {
        const probe = document.createElement("div");
        document.body.append(probe);
        const tok = (t: string): string => {
          probe.style.color = `var(--color-xfire-${t})`;
          return getComputedStyle(probe).color;
        };
        const f = document.querySelector<HTMLInputElement>(sel);
        const sel2 = f ? getComputedStyle(f, "::selection") : null;
        const out = {
          focus: document.documentElement.dataset.windowFocus ?? "unset",
          bg: sel2?.backgroundColor ?? "",
          fg: sel2?.color ?? "",
          want: {
            active: tok("input-selection-bg"), activeText: tok("selection-text"),
            inactive: tok("input-selection-bg-inactive"), inactiveText: tok("input-text"),
          },
        };
        probe.remove();
        return out;
      }, field)) as { focus: string; bg: string; fg: string; want: Record<string, string> };
    // Both rules are keyed on <html data-window-focus> (window_focus.ts sets
    // it from the window's focus and blur). The OS can hold the window
    // unfocused (a locked screen, another app in front), so each sample sets
    // the state it measures, and the real one is restored after.
    const realFocus = (await browser.execute(() => document.documentElement.dataset.windowFocus ?? "")) as string;
    console.log(`[st1] selection: the window's own focus state is "${realFocus}"`);
    await browser.execute(() => {
      document.documentElement.dataset.windowFocus = "active";
    });
    await selectAll("before the focused sample");
    const focused = await read();
    const focusedPaint = await paintedSelection(field, "selection-focused-field");
    await shot("selection-focused");
    await browser.execute(() => {
      document.documentElement.dataset.windowFocus = "inactive";
    });
    await selectAll("before the unfocused sample");
    const unfocused = await read();
    const unfocusedPaint = await paintedSelection(field, "selection-unfocused-field");
    await shot("selection-unfocused");
    await browser.execute((was: string) => {
      document.documentElement.dataset.windowFocus = was;
    }, realFocus);
    console.log(`[st1] selection: focused ${JSON.stringify(focused)}; unfocused ${JSON.stringify(unfocused)}`);
    console.log(`[st1] selection painted: focused ${JSON.stringify(focusedPaint)}; unfocused ${JSON.stringify(unfocusedPaint)}`);
    expect({ focus: focused.focus, fg: focused.fg }).toEqual({ focus: "active", fg: focused.want.activeText });
    expect({ focus: unfocused.focus, fg: unfocused.fg }).toEqual({ focus: "inactive", fg: unfocused.want.inactiveText });
    // What the user sees: the painted highlight is the token itself.
    expect({ focused: focusedPaint.painted, unfocused: unfocusedPaint.painted }).toEqual({ focused: focused.want.active, unfocused: unfocused.want.inactive });
  });

  it("a dialog outside Settings (Add Friend) takes the same text-field roles", async () => {
    await switchTo(browser, BUDDY);
    await openAddFriend(browser);
    const field = await colours([
      { what: "Add Friend field ground", selector: ".form-input", property: "background-color", token: "input-bg" },
      { what: "Add Friend field text", selector: ".form-input", property: "color", token: "input-text" },
      { what: "Add Friend field outline", selector: ".form-input", property: "border-top-color", token: "progress-outline" },
    ]);
    console.log(`[st1] add friend: ${JSON.stringify(field.read)}`);
    await shot("add-friend-field");
    await closeModal(browser);
    expect(field.mismatched).toEqual([]);
  });
});
