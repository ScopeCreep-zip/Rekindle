// Plan E7.14 S5 (moved from e2e/skin-apply.spec.ts, E7.7.2 "Apply"): the
// login window shows the device's login skin the way every window shows a
// skin, its `:root[data-skin-base]` and its one `link#rekindle-skin` sheet,
// and it follows a change. A user chooses the login skin in Settings →
// Appearance ("Use as login skin"); the login window is what they see
// after logging out. The mocked spec pushed a golden SkinChanged event;
// here the real Host sends it.

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, switchTo, waitUntil } from "../../../walk/helpers.ts";
import { LOGIN, openSettings } from "../../../walk/nav.ts";
import { applySkin, createIdentity, logOut, newUser, setLoginSkin, unlock, type User } from "../../journeys.ts";
import { builtinPacks } from "../../skin-pack.ts";
import { shot } from "../../shots.ts";

/** What the window `browser` is in applied: its base and its sheet, once loaded. */
const APPLIED = 'var l = document.getElementById("rekindle-skin"); return { base: document.documentElement.dataset.skinBase || "", href: l && l.sheet ? l.getAttribute("href") : "" };';

/** The login window shows `base` and `href`. */
async function loginShows(base: string, href: string): Promise<void> {
  await switchTo(browser, LOGIN, NET_WAIT_MS);
  let now = { base: "", href: "" };
  await waitUntil(
    async () => {
      now = (await browser.execute(APPLIED)) as typeof now;
      return now.base === base && now.href === href;
    },
    NET_WAIT_MS,
    "",
  ).catch(() => undefined);
  expect(now).toEqual({ base, href });
}

describe("the login window wears the login skin and follows a change", function () {
  this.timeout(600_000);
  const me: User = newUser("loginskin");
  // A light skin, then a dark one, so the base must change with the sheet.
  const light = builtinPacks().find((p) => p.variants.some((v) => v.base === "light"));
  const dark = builtinPacks().find((p) => p.file !== "symbiosis" && p.variants.every((v) => v.base === "dark"));

  before(async () => {
    if (!light || !dark) throw new Error("the built-in skins hold no light skin or no dark skin other than Symbiosis");
    await createIdentity(browser, me);
    await openSettings(browser, "Appearance");
  });

  it("the login skin chosen in Settings is what the login window shows after logging out", async () => {
    if (!light) return;
    const href = await applySkin(browser, me, light.name);
    await setLoginSkin(browser, me, light.name);
    await logOut(browser, me);
    await loginShows("light", href);
    await shot("login-light");
  });

  it("a newly chosen login skin replaces it", async () => {
    if (!dark) return;
    await unlock(browser, me);
    await openSettings(browser, "Appearance");
    const href = await applySkin(browser, me, dark.name);
    await setLoginSkin(browser, me, dark.name);
    await logOut(browser, me);
    await loginShows("dark", href);
    await shot("login-dark");
  });
});
