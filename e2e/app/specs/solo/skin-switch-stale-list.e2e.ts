import { browser, expect } from "@wdio/globals";
import { resolveIntent, shown, waitUntil } from "../../../walk/helpers.ts";
import { openSettings } from "../../../walk/nav.ts";
import { createIdentity, newUser, type User } from "../../journeys.ts";

interface HeldSkinList {
  active?: string;
  /** The held reply's skin names, recorded when it was held. */
  heldNames?: string[];
  /** Set when the held reply was let go (after the second choice landed). */
  released: boolean;
  consumed: boolean;
  release?: () => void;
  restore: () => void;
}

type ProbeWindow = Window & { heldSkinList?: HeldSkinList };

async function selectSkin(name: string, me: User) {
  // The names come from the current app's built-in radios.
  const label = await browser
    .$$(".skin-row label.skin-choice")
    .find(async (candidate) => (await (await candidate.$(".skin-name")).getText()) === name);
  if (!label) throw new Error(`No skin radio named ${name}`);
  await (await label.$("input[type=radio]")).click();
  await resolveIntent(browser, { passphrase: me.passphrase });
}

async function currentSkin() {
  return browser.execute(() => document.querySelector(".skin-row-shown .skin-name")?.textContent?.trim());
}

describe("Settings Appearance handles a late skin library response", function () {
  this.timeout(180_000);
  const me: User = { ...newUser("skin-order"), displayName: "Ari" };

  it("keeps the newest skin when a previous list response arrives later", async () => {
    await createIdentity(browser, me);
    await openSettings(browser, "Appearance");
    await shown(browser, ".skin-row-shown .skin-name");

    const original = await currentSkin();
    const choices = await browser.execute(() =>
      Array.from(document.querySelectorAll(".skin-row .skin-name"), (node) => node.textContent?.trim() ?? ""),
    );
    const [first, second] = choices.filter((name) => name && name !== original && !name.includes("Accessible"));
    if (!first || !second) throw new Error("Expected two other built-in skins");

    await browser.execute((expected: string) => {
      const page = window as ProbeWindow;
      const fetch = page.fetch;
      const realFetch = fetch.bind(page);
      const state: HeldSkinList = { released: false, consumed: false, restore: () => { page.fetch = fetch; } };
      page.heldSkinList = state;
      page.fetch = async (input, init) => {
        const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
        const response = await realFetch(input, init);
        if (!url.endsWith("/list_skins") || !response.ok || state.release) return response;
        // A Host command answers in its Done envelope ({ value, undo }:
        // src/ipc/invoke.ts hostInvoke); the library is its value.
        const done = (await response.clone().json()) as {
          value?: { active: string; skins: { id: string; name: string }[] };
        };
        const library = done.value;
        if (state.release || !library?.skins?.some((skin) => skin.id === library.active && skin.name === expected)) {
          return response;
        }

        // Hold a real response from the first switch and leave every later request flowing.
        state.active = library.active;
        state.heldNames = library.skins.map((skin) => skin.name);
        const json = response.json.bind(response);
        response.json = async () => {
          const body = await json();
          // Tauri consumes JSON, then fulfills the pending invoke in microtasks.
          // The following frame runs after that fulfillment and the Solid update.
          requestAnimationFrame(() => { state.consumed = true; });
          return body;
        };
        await new Promise<void>((release) => { state.release = release; });
        return response;
      };
    }, first);
    try {
      await selectSkin(first, me);
      await waitUntil(
        () => browser.execute(() => Boolean((window as ProbeWindow).heldSkinList?.active)),
        20_000,
        "first switch did not produce a real list_skins response",
      );

      await selectSkin(second, me);
      await waitUntil(() => currentSkin().then((name) => name === second), 30_000, `${second} did not become current`);
      await browser.execute(() => {
        const held = (window as ProbeWindow).heldSkinList;
        if (held?.release) held.released = true;
        held?.release?.();
      });
      await waitUntil(
        () => browser.execute(() => (window as ProbeWindow).heldSkinList?.consumed === true),
        10_000,
        "Tauri did not consume the delayed response",
      );
      // The fixture's own guard: a run that never held a real reply, or never
      // let it go after the second choice, is "fixture did not hold", never a
      // verdict on the app.
      const fixture = (await browser.execute(() => {
        const held = (window as ProbeWindow).heldSkinList;
        return { active: held?.active ?? null, names: held?.heldNames ?? [], released: held?.released ?? false, consumed: held?.consumed ?? false };
      })) as { active: string | null; names: string[]; released: boolean; consumed: boolean };
      console.log(`[stale] held reply: active ${fixture.active}, ${fixture.names.length} skins (${fixture.names.slice(0, 4).join(", ")}…), released ${fixture.released}, consumed ${fixture.consumed}; first ${first}, second ${second}`);
      if (!fixture.active || fixture.names.length === 0 || !fixture.released || !fixture.consumed) {
        throw new Error(`fixture did not hold: ${JSON.stringify(fixture)}`);
      }
      const shown = await currentSkin();
      console.log(`[stale] in use after the old reply landed: ${shown} (want ${second})`);
      expect(shown).toBe(second);
    } finally {
      await browser.execute(() => {
        const page = window as ProbeWindow;
        page.heldSkinList?.release?.();
        page.heldSkinList?.restore();
        delete page.heldSkinList;
      });
    }
  });
});
