// FE-LINT slice C (plan E7.3, wiring guide §13): controls that were clickable
// divs and spans are native buttons (or labels and fieldsets), so the
// keyboard reaches and works them. Two cases:
// 1. The look: the community window (a category header, the channel topic)
//    and Settings → Roles (a role's summary row) in Symbiosis and in the
//    light Alloy, saved by name so a run before the change and a run after
//    it compare shot by shot (only the look; run alone on the old tree).
// 2. The keyboard: Tab reaches each control with a visible focus ring, and
//    Enter and Space work it as a click does. On macOS, WebKit's Tab reaches
//    only text fields and lists unless the system's Keyboard navigation is on
//    (AppleKeyboardUIMode), an OS setting this suite leaves alone: there each
//    control is focused directly and Enter/Space are proven, and Tab reach
//    with the ring is the Linux run's (WebKitGTK tabs to every control).

import { browser, expect } from "@wdio/globals";
import type { Browser } from "webdriverio";
import { NET_WAIT_MS, byText, gone, resolveIntent, shown, typeInto, waitUntil } from "../../../walk/helpers.ts";
import { communitySettings, createCommunity, createIdentity, openCommunity, selectChannel, setCommunityName, setLocationSharing, type User } from "../../journeys.ts";
import { applySkin, skinShown } from "../../journeys_skins.ts";
import { openSettings } from "../../../walk/nav.ts";
import { shot } from "../../shots.ts";

const b = browser as unknown as Browser;
// Fixed names: the before and after runs draw the same text.
const ada: User = { displayName: "Ada", passphrase: "a11y-controls-passphrase-1" };
const PLACE = "Keyboard Hall";
const ROLE = "Greeter";

/** Close Community Settings, back to the window. */
async function closeSettings(): Promise<void> {
  await b.keys("Escape");
  await gone(b, ".modal-container .form-tabs", NET_WAIT_MS);
}

/** The community window's content ground: the first opaque background under
 *  the middle of the message area, the element that paints it, and every
 *  theme colour token that resolves to that colour in this window. */
interface Ground {
  bg: string;
  painter: string;
  tokens: Record<string, string>;
}

async function groundOfContent(): Promise<Ground> {
  return (await b.execute(() => {
    const area = document.querySelector(".community-main")!.getBoundingClientRect();
    let el: Element | null = document.elementFromPoint(area.left + area.width / 2, area.top + area.height / 2);
    let bg = "";
    for (; el; el = el.parentElement) {
      const c = getComputedStyle(el).backgroundColor;
      if (c && !/rgba\([^)]*,\s*0\)$/.test(c) && c !== "transparent") {
        bg = c;
        break;
      }
    }
    const painter = el ? `${el.tagName.toLowerCase()}.${typeof el.className === "string" ? el.className.split(/\s+/).slice(0, 2).join(".") : ""}` : "none";
    // Every --color-xfire-* token the sheets name, resolved in this window.
    const names = new Set<string>();
    for (const sheet of Array.from(document.styleSheets)) {
      let rules: CSSRuleList;
      try {
        rules = sheet.cssRules;
      } catch {
        continue;
      }
      for (const m of Array.from(rules).map((r) => r.cssText).join("\n").matchAll(/--color-xfire-[a-z0-9-]+/g)) names.add(m[0]);
    }
    const probe = document.createElement("span");
    document.body.appendChild(probe);
    const tokens: Record<string, string> = {};
    for (const name of names) {
      probe.style.color = `var(${name})`;
      tokens[name] = getComputedStyle(probe).color;
    }
    probe.remove();
    return { bg, painter, tokens };
  })) as Ground;
}

/** The community window and its Roles tab, saved under `skin`'s name. */
async function shotsIn(skin: string, href: string): Promise<Ground> {
  await openCommunity(b, PLACE);
  await skinShown(b, href);
  await selectChannel(b, "general");
  await shown(b, ".category-header");
  await byText(b, ".member-item .member-name", ada.displayName, NET_WAIT_MS);
  await byText(b, ".channel-presence-badge", "1", NET_WAIT_MS);
  await gone(b, ".governance-publication", NET_WAIT_MS);
  await gone(b, ".toast-item", NET_WAIT_MS);
  const ground = await groundOfContent();
  await shot(`community-${skin}`);
  await communitySettings(b, "Roles");
  await byText(b, ".settings-role-row", ROLE, NET_WAIT_MS);
  await gone(b, ".toast-item", NET_WAIT_MS);
  await shot(`roles-${skin}`);
  await closeSettings();
  return ground;
}

/** Whether the app runs on macOS, where Tab skips buttons (see the header). */
async function onMac(): Promise<boolean> {
  return (await b.execute(() => navigator.platform.startsWith("Mac"))) as boolean;
}

/**
 * Bring focus to `selector` (optionally holding `text`): by Tab from where
 * focus is, at most 80 presses, with its focus ring showing (`:focus-visible`
 * and a drawn outline); on macOS by focusing it directly (Tab reach is the
 * Linux run's), its ring then reported as `macOS: focused directly`.
 */
async function tabTo(selector: string, text = ""): Promise<{ ring: string }> {
  if (await onMac()) {
    const focused = (await b.execute(
      (sel: string, want: string) => {
        const el = Array.from(document.querySelectorAll<HTMLElement>(sel)).find((e) => (e.textContent ?? "").includes(want));
        el?.focus();
        return el !== undefined && document.activeElement === el;
      },
      selector,
      text,
    )) as boolean;
    if (!focused) throw new Error(`${selector}${text ? ` "${text}"` : ""} could not take focus`);
    return { ring: "macOS: focused directly" };
  }
  const trail: string[] = [];
  for (let i = 0; i < 80; i++) {
    await b.keys("Tab");
    trail.push(
      (await b.execute(() => {
        const el = document.activeElement as HTMLElement | null;
        if (!el || el === document.body) return `body(hasFocus=${document.hasFocus()})`;
        const cls = typeof el.className === "string" ? el.className.split(/\s+/).slice(0, 2).join(".") : "";
        return `${el.tagName.toLowerCase()}${cls ? `.${cls}` : ""}`;
      })) as string,
    );
    const at = (await b.execute(
      (sel: string, want: string) => {
        const el = document.activeElement as HTMLElement | null;
        if (!el?.matches(sel) || !(el.textContent ?? "").includes(want)) return null;
        const style = getComputedStyle(el);
        return {
          visible: el.matches(":focus-visible"),
          ring: style.outlineStyle === "none" || style.outlineWidth === "0px" ? "" : `${style.outlineWidth} ${style.outlineStyle} ${style.outlineColor}`,
        };
      },
      selector,
      text,
    )) as { visible: boolean; ring: string } | null;
    if (at) {
      if (!at.visible || at.ring === "") throw new Error(`${selector} has focus but no visible focus ring (${JSON.stringify(at)})`);
      return { ring: at.ring };
    }
  }
  throw new Error(`Tab never reached ${selector}${text ? ` "${text}"` : ""}; focus after each Tab: ${trail.join(" > ")}`);
}

describe("converted controls (FE-LINT slice C)", function () {
  this.timeout(900_000);

  it("look as before in Symbiosis and Alloy", async () => {
    await createIdentity(b, ada);
    await createCommunity(b, ada, PLACE);
    await openCommunity(b, PLACE);
    await setCommunityName(b, ada, ada.displayName);
    await setLocationSharing(b, ada, true);
    await communitySettings(b, "Roles");
    await (await byText(b, ".modal-container .form-btn-secondary", "Create Role")).click();
    await typeInto(await shown(b, 'input[placeholder="Role name..."]'), ROLE);
    await (await byText(b, ".modal-container .form-btn-primary", "Create")).click();
    await resolveIntent(b, { passphrase: ada.passphrase, appearMs: 3000 });
    await byText(b, ".settings-role-row", ROLE, NET_WAIT_MS);
    await closeSettings();

    await openSettings(b, "Appearance");
    const symbiosis = await applySkin(b, ada, "Symbiosis");
    const darkGround = await shotsIn("symbiosis", symbiosis);
    await openSettings(b, "Appearance");
    const alloy = await applySkin(b, ada, "Alloy");
    expect(alloy).not.toBe(symbiosis);
    const lightGround = await shotsIn("alloy", alloy);

    // The content's ground follows its token under each skin: the tokens that
    // paint it under Symbiosis must paint it under Alloy too, at Alloy's
    // values. A ground that stays Symbiosis's while its token changed is the
    // window not taking the skin (a product fault, not capture timing).
    const paintedBy = Object.keys(darkGround.tokens).filter((t) => darkGround.tokens[t] === darkGround.bg);
    const follows = paintedBy.filter((t) => lightGround.tokens[t] === lightGround.bg);
    const changed = paintedBy.filter((t) => lightGround.tokens[t] !== darkGround.tokens[t]);
    console.log(
      `[a11y] content ground: Symbiosis ${darkGround.bg} (${darkGround.painter}), Alloy ${lightGround.bg} (${lightGround.painter}); ` +
        `painted by ${JSON.stringify(paintedBy)}; Alloy's values ${JSON.stringify(Object.fromEntries(paintedBy.map((t) => [t, lightGround.tokens[t]])))}; ` +
        `following under Alloy ${JSON.stringify(follows)}; changed by Alloy ${JSON.stringify(changed)}`,
    );
    expect({ paintedByAToken: paintedBy.length > 0, followsUnderAlloy: follows.length > 0 }).toEqual({ paintedByAToken: true, followsUnderAlloy: true });
  });

  it("are reached by Tab with a visible ring and work with Enter and Space", async () => {
    await openCommunity(b, PLACE);
    const header = ".category-header";
    // The window holds several category headers (Servers, Upcoming): the one
    // under test is the one with focus.
    const expanded = async (): Promise<string | null> =>
      (await b.execute(() => document.activeElement?.getAttribute("aria-expanded") ?? null)) as string | null;
    if (!(await onMac())) {
      // Give the community webview keyboard focus before the first Tab.
      await (await shown(b, ".community-channel-header-name")).click();
    }

    // A category header folds on Enter and unfolds on Space.
    const headerRing = await tabTo(header);
    const before = await expanded();
    await b.keys("Enter");
    await waitUntil(async () => (await expanded()) !== before, NET_WAIT_MS, "Enter did not fold the category");
    await b.keys(" ");
    await waitUntil(async () => (await expanded()) === before, NET_WAIT_MS, "Space did not unfold the category");
    console.log(`[a11y] category header: ring ${headerRing.ring}; Enter folds, Space unfolds (aria-expanded ${before})`);
    await shot("keyboard-category-header");

    // The channel topic opens its editor on Enter; Escape leaves it.
    const topicRing = await tabTo(".channel-topic");
    await b.keys("Enter");
    await shown(b, ".channel-topic-header-input");
    console.log(`[a11y] channel topic: ring ${topicRing.ring}; Enter opens the topic editor`);
    await shot("keyboard-topic-editor");
    await b.keys("Escape");

    // A role's summary opens its editor on Space.
    // The Roles tab click leaves focus inside the modal for Tab navigation.
    await communitySettings(b, "Roles");
    const roleRing = await tabTo(".settings-role-summary", ROLE);
    await b.keys(" ");
    await shown(b, ".settings-section-accented input.form-input");
    console.log(`[a11y] role summary: ring ${roleRing.ring}; Space opens the role editor`);
    await shot("keyboard-role-editor");

    expect({ header: headerRing.ring !== "", topic: topicRing.ring !== "", role: roleRing.ring !== "" }).toEqual({ header: true, topic: true, role: true });
  });
});
