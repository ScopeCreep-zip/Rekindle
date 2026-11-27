// Probe (lead, 2026-11-26): Linux and the Mac each lost every third skin
// switch in settings-theme (switches 5, 8 and 11: Space Cowboy, xSLIMFIRE,
// XGlow), the new pack's radio checked while "in use" stayed on the old one.
// This switches through every built-in in turn from Settings → Appearance,
// as a user does (the radio, then the Host's intent review), and records for
// each switch: whether and when the review appears (up to 10 s, where
// applySkin waited 1.5 s), its title and outcome, whether the pack becomes
// the one in use within 30 s, and on a miss, what the page holds then (the
// radio's state, a late review, the row shown in use). A miss is counted,
// never thrown, so one run gives the cadence.

import { browser, expect } from "@wdio/globals";
import { resolveIntent, switchTo, waitUntil } from "../../../walk/helpers.ts";
import { SETTINGS, openSettings } from "../../../walk/nav.ts";
import { createIdentity, newUser, type User } from "../../journeys.ts";
import { shot } from "../../shots.ts";
import { ACCESSIBLE_VARIANT, builtinPacks } from "../../skin-pack.ts";

const PACKS = builtinPacks().filter((p) => p.file !== ACCESSIBLE_VARIANT.file);

async function inUse(): Promise<string> {
  return ((await browser.execute(() => document.querySelector(".skin-row-shown .skin-name")?.textContent?.trim() ?? "")) as string);
}

describe("skin switches in a row: does every one apply? (probe)", function () {
  this.timeout(2_400_000);
  const me: User = { ...newUser("cadence"), displayName: "Ari" };

  before(async () => {
    await createIdentity(browser, me);
    await openSettings(browser, "Appearance");
  });

  it("switches through every built-in twice, each switch's review and outcome logged", async () => {
    const missed: string[] = [];
    let n = 0;
    for (const round of [1, 2]) {
      for (const pack of PACKS) {
        n += 1;
        await switchTo(browser, SETTINGS);
        if ((await inUse()) === pack.name) continue;
        const before = await inUse();
        const radio = `//*[contains(concat(" ", normalize-space(@class), " "), " skin-row ")][.//*[contains(concat(" ", normalize-space(@class), " "), " skin-name ") and normalize-space(.)="${pack.name}"]]//input[@type="radio"]`;
        const t0 = Date.now();
        await (await browser.$(radio)).click();
        // The review: when it appears, if it does, up to 10 s.
        let reviewMs = -1;
        await waitUntil(async () => {
          if (await browser.$(".intent-dialog").isDisplayed().catch(() => false)) {
            reviewMs = Date.now() - t0;
            return true;
          }
          return false;
        }, 10_000, "").catch(() => undefined);
        const title = reviewMs >= 0
          ? ((await browser.execute(() => document.querySelector(".intent-dialog")?.textContent?.replace(/\s+/g, " ").trim().slice(0, 120) ?? "")) as string)
          : "";
        const outcome = reviewMs >= 0 ? await resolveIntent(browser, { passphrase: me.passphrase, appearMs: 1000 }) : "none";
        const applied = await waitUntil(async () => (await inUse()) === pack.name, 30_000, "").then(() => true, () => false);
        const appliedMs = Date.now() - t0;
        let state = "";
        if (!applied) {
          state = (await browser.execute((name: string) => {
            const row = Array.from(document.querySelectorAll(".skin-row")).find((r) => r.querySelector(".skin-name")?.textContent?.trim() === name);
            const r = row?.querySelector<HTMLInputElement>("input[type=radio]");
            return JSON.stringify({
              radioChecked: r?.checked ?? null,
              lateReview: !!document.querySelector(".intent-dialog"),
              shown: document.querySelector(".skin-row-shown .skin-name")?.textContent?.trim() ?? "",
              toasts: Array.from(document.querySelectorAll(".toast-item")).map((t) => t.textContent?.trim()).slice(0, 3),
            });
          }, pack.name)) as string;
          missed.push(`#${n} ${pack.file}`);
          await shot(`missed-${n}-${pack.file}`);
          // A late review left open is answered, so the next switch starts clean.
          if (await browser.$(".intent-dialog").isDisplayed().catch(() => false)) {
            console.log(`[cadence] #${n} late review answered: ${await resolveIntent(browser, { passphrase: me.passphrase, appearMs: 500 })}`);
          }
        }
        console.log(`[cadence] round ${round} #${n} ${before} -> ${pack.name}: review ${reviewMs} ms (${title || "none"}), outcome ${outcome}, applied ${applied} after ${appliedMs} ms ${state}`);
      }
    }
    console.log(`[cadence] missed: ${JSON.stringify(missed)}`);
    expect(missed).toEqual([]);
  });
});
