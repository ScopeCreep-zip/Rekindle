// Plan E7.14 S5: every built-in skin, applied for real through Settings →
// Appearance, in every kind of window over real data (friends from a peer
// fleet, a 1:1 chat, a group chat, a community, a profile, Settings). For
// each window it checks that the skin's stylesheet is the one loaded and
// that every colour the skin declares (its package's DTCG tokens,
// skin-pack.ts) is the colour the window computes; and that the skin
// changes at least one colour from its base's defaults, since a skin that
// changes none looks like the default theme with images swapped (the
// owner's "skins only partly applied", 2026-11-19). Each window is saved
// as e2e/app/out/skins/<skin>/<window>.png with its verdict in
// e2e/app/out/skins/applied.txt, the layout of the 2026-11-19 capture, so
// runs compare.

import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { browser, expect } from "@wdio/globals";
import type { Browser } from "webdriverio";
import { type SkinTokenId, tokenVar } from "../../../../src/styles/generated/tokens.ts";
import { NET_WAIT_MS, WAIT_MS, byText, press, resolveIntent, shown, switchTo, typeAndSend, waitForHandle, waitUntil } from "../../../walk/helpers.ts";
import { BUDDY, SETTINGS, openSettings } from "../../../walk/nav.ts";
import { type Peer, peerAcceptGroup, peerSendDm, peerSendGroup, startFleet, stopFleet } from "../../fleet.ts";
import {
  createCommunity,
  createGroup,
  createIdentity,
  createTextChannel,
  friendRow,
  newUser,
  openChat,
  openCommunity,
  openGroup,
  postMessage,
  seesMessage,
  selectChannel,
  type User,
} from "../../journeys.ts";
import { type SkinPack, baseDefaults, builtinPacks } from "../../skin-pack.ts";
import { befriendFleet } from "../../social.ts";
import { BUDDY_LIST, CHAT_DM, CHAT_GROUP, COMMUNITY, PROFILE, SETTINGS_APPEARANCE, SETTINGS_PROFILE, type WindowCase, sizeWindow } from "../../windows.ts";

const OUT = path.join(import.meta.dirname, "..", "..", "out", "skins");
const PACKS = builtinPacks();

/**
 * Known failure, held strictly: these packages declare no colours yet, so
 * they look like the default theme with their images swapped. Plan E7.7.7
 * imports each skin's own colours (wt/e777l onward; with it all 11 pass,
 * S5 run on 423ff2d53, 2026-11-20). Until it lands, each must show exactly
 * the `changes` finding; one that declares colours fails, saying to drop
 * it from this list.
 */
const KNOWN_UNCOLOURED = new Set(["onyx", "spacecowboy", "stealthx", "total-chaos", "xglow"]);

function slug(s: string): string {
  return s.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
}

/** The custom property behind a DTCG token id, or `null` for an id outside the registry. */
function cssProperty(id: string): string | null {
  const v = tokenVar(id as SkinTokenId) as string;
  return v.includes("undefined") ? null : v.slice(4, -1);
}

interface Shown {
  base: string;
  href: string;
  loaded: boolean;
  /** Declared colours the window computes differently: token → [declared, shown]. */
  mismatches: Record<string, [string, string]>;
}

/**
 * What the window `b` is in shows of `colours` ([token, property, value]):
 * each declared value and the property's value, both resolved by the
 * engine through one probe element's `color`, so `#rrggbb` and `rgb()`
 * compare. Sent as a script body; nothing is evaluated from the page.
 */
async function windowShows(b: Browser, colours: [string, string, string][]): Promise<Shown> {
  return (await b.execute(`var want = ${JSON.stringify(colours)};
var probe = document.createElement("div");
probe.style.position = "fixed";
document.body.appendChild(probe);
var mismatches = {};
for (var i = 0; i < want.length; i++) {
  probe.style.color = want[i][2];
  var declared = getComputedStyle(probe).color;
  probe.style.color = "var(" + want[i][1] + ")";
  var shown = getComputedStyle(probe).color;
  if (declared !== shown) mismatches[want[i][0]] = [declared, shown];
}
probe.remove();
var link = document.getElementById("rekindle-skin");
return { base: document.documentElement.dataset.skinBase || "", href: link ? link.getAttribute("href") : "", loaded: !!(link && link.sheet), mismatches: mismatches };`)) as Shown;
}

describe("every built-in skin, applied for real", function () {
  this.timeout(1_800_000);
  const me: User = { ...newUser("skins") };
  let peers: Peer[] = [];
  const handles: Record<string, string> = {};
  const report: string[] = [];

  before(async () => {
    mkdirSync(OUT, { recursive: true });
    await createIdentity(browser, me);
    peers = await startFleet(["PeerOne", "PeerTwo"]);
    const myKey = await befriendFleet(browser, me, peers);

    await peerSendDm(peers[0], myKey, "Hey, are you on tonight?");
    await openChat(browser, peers[0].displayName);
    await seesMessage(browser, me, "are you on tonight");
    await typeAndSend(browser, ".message-input-field", "Yes, after nine. Bring the others.");
    handles.dm = await browser.getWindowHandle();

    const group = "Tonight's squad";
    await createGroup(browser, me, group, peers.map((p) => p.displayName));
    const id = await peerAcceptGroup(peers[0], group);
    await peerAcceptGroup(peers[1], group);
    await peerSendGroup(peers[0], id, "Who's bringing the map pack?");
    await openGroup(browser, group);
    await seesMessage(browser, me, "map pack");
    await typeAndSend(browser, ".message-input-field", "I have it, sending after dinner.");
    handles.group = await browser.getWindowHandle();

    await createCommunity(browser, me, "Skins Test Community");
    await openCommunity(browser, "Skins Test Community");
    await createTextChannel(browser, me, "general-chat");
    await selectChannel(browser, "general-chat");
    for (const m of ["Welcome to the community.", "Server is up on the usual port.", "Patch notes are pinned."]) await postMessage(browser, me, m);
    handles.community = await browser.getWindowHandle();

    const row = await friendRow(browser, peers[1].displayName);
    await browser.execute((node: HTMLElement) => {
      const r = node.getBoundingClientRect();
      node.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2, clientX: r.x + 5, clientY: r.y + 5 }));
    }, row as unknown as HTMLElement);
    await press(browser, await byText(browser, ".context-menu-item", "View Profile"));
    handles.profile = await waitForHandle(browser, (h) => h.startsWith("profile-"));
    await browser.switchToWindow(handles.profile);
    await shown(browser, ".profile-actions");

    await openSettings(browser, "Appearance");
    await shown(browser, ".skin-row", NET_WAIT_MS);
  });

  after(async () => {
    mkdirSync(OUT, { recursive: true });
    writeFileSync(path.join(OUT, "applied.txt"), `${report.join("\n")}\n`);
    await stopFleet(peers);
  });

  it("Settings → Appearance lists the shipped built-in skins, in order", async () => {
    await switchTo(browser, SETTINGS);
    const listed: string[] = [];
    for (const r of await browser.$$(".skin-row")) listed.push((await (await r.$(".skin-name")).getText()).trim());
    report.push(`listed: ${listed.join(", ")}`);
    expect(listed).toEqual(PACKS.map((p) => p.name));
  });

  for (const pack of PACKS) {
    const known = KNOWN_UNCOLOURED.has(pack.file);
    it(`${pack.name}: every window shows the colours it declares, and it changes its base${known ? " (known failure: no colours until E7.7.7)" : ""}`, async () => {
      const found = await applyAndCheck(pack);
      if (!known) {
        expect(found).toEqual({});
        return;
      }
      if (pack.variants.some((v) => Object.keys(v.colours).length > 0)) {
        throw new Error(`${pack.file}.rkskin now declares colours: drop it from KNOWN_UNCOLOURED in skins.e2e.ts`);
      }
      expect(Object.keys(found)).toEqual(["changes"]);
    });
  }

  /** Apply `pack` from Settings, then check and save each window; findings keyed by window. */
  async function applyAndCheck(pack: SkinPack): Promise<Record<string, unknown>> {
    const found: Record<string, unknown> = {};
    const dir = path.join(OUT, slug(pack.name));
    mkdirSync(dir, { recursive: true });
    await switchTo(browser, SETTINGS);
    await (await byText(browser, ".form-tab", "Appearance")).click();
    // Coming back from the Profile tab mounts Appearance afresh, and its skin
    // list renders once the list loads: wait for the row in use.
    await shown(browser, ".skin-row-shown .skin-name", NET_WAIT_MS);
    const inUse = async (): Promise<string> => (await (await browser.$(".skin-row-shown .skin-name")).getText()).trim();
    if ((await inUse()) !== pack.name) {
      for (const r of await browser.$$(".skin-row")) {
        if ((await (await r.$(".skin-name")).getText()).trim() !== pack.name) continue;
        await (await r.$("input[type=radio]")).click();
        await resolveIntent(browser, { passphrase: me.passphrase, appearMs: 1500 });
        break;
      }
      await waitUntil(async () => (await inUse()) === pack.name, NET_WAIT_MS, `${pack.name} never shown as the skin in use`);
    }

    // The skin already in use is not clicked, so nothing above waited for
    // this window's sheet (the 2026-11-20 capture read "no link" here for
    // Symbiosis): wait for it before taking it as the one every window shows.
    let settingsNow = await windowShows(browser, []);
    await waitUntil(
      async () => {
        settingsNow = await windowShows(browser, []);
        return settingsNow.href.startsWith("skin://") && settingsNow.loaded;
      },
      NET_WAIT_MS,
      "Settings never linked a skin sheet",
    );
    const variant = pack.variants.find((v) => v.base === settingsNow.base);
    if (!variant) return { variant: `no ${settingsNow.base} variant, but the window's base is ${settingsNow.base}` };
    const colours: [string, string, string][] = [];
    for (const [id, value] of Object.entries(variant.colours)) {
      const property = cssProperty(id);
      if (property === null) found[`token ${id}`] = "declared, but not in the skin registry";
      else colours.push([id, property, value]);
    }
    const defaults = baseDefaults(variant.base);
    const changed = colours.filter(([, property, value]) => defaults[property] !== value).map(([id]) => id);
    if (changed.length === 0) {
      found.changes = `declares ${colours.length} colour(s), none different from the ${variant.base} defaults: the skin looks like the default theme`;
    }
    report.push(`== ${pack.name} (${pack.file}.rkskin, ${variant.base}): ${colours.length} colours declared, ${changed.length} change the base [${changed.join(" ")}]; sheet ${settingsNow.href}`);

    const check = async (file: string, w: WindowCase): Promise<void> => {
      await sizeWindow(browser, w.def);
      let now = await windowShows(browser, colours);
      // Each window applies SkinChanged itself; wait for this one to show the chosen sheet.
      await waitUntil(
        async () => {
          now = await windowShows(browser, colours);
          return now.href === settingsNow.href && now.loaded && Object.keys(now.mismatches).length === 0;
        },
        WAIT_MS,
        "",
      ).catch(() => undefined);
      await browser.saveScreenshot(path.join(dir, `${file}.png`));
      const applied = now.href === settingsNow.href && now.loaded;
      const wrong = Object.keys(now.mismatches).length;
      report.push(`${slug(pack.name)}/${file}.png  ${applied ? "APPLIED" : "NOT APPLIED"}  ${wrong === 0 ? "colours ok" : `${wrong} colour(s) wrong`}  base=${now.base} href=${now.href}`);
      if (!applied) found[`${file}: sheet`] = { want: settingsNow.href, shown: now.href, loaded: now.loaded };
      if (wrong > 0) found[`${file}: declared colours not shown`] = now.mismatches;
    };

    await check("settings-appearance", SETTINGS_APPEARANCE);
    await (await byText(browser, ".form-tab", "Profile")).click();
    await check("settings-profile", SETTINGS_PROFILE);

    await switchTo(browser, BUDDY);
    await (await friendRow(browser, peers[0].displayName)).click();
    await check("buddy-list-friends", BUDDY_LIST);
    await (await browser.$('.buddy-tab[title^="Communities"]')).click();
    await check("buddy-list-communities", BUDDY_LIST);
    await (await browser.$('.buddy-tab[title^="Friends"]')).click();

    for (const [name, file, w] of [
      ["dm", "chat-dm", CHAT_DM],
      ["group", "chat-group", CHAT_GROUP],
      ["community", "community", COMMUNITY],
      ["profile", "profile", PROFILE],
    ] as const) {
      await browser.switchToWindow(handles[name]);
      await check(file, w);
    }
    return found;
  }
});
