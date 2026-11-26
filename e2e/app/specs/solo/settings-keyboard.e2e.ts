import { browser, expect } from "@wdio/globals";
import { Key } from "webdriverio";
import { openSettings } from "../../../walk/nav.ts";
import { createIdentity, newUser } from "../../journeys.ts";
import { shot } from "../../shots.ts";

async function expectTab(id: string): Promise<void> {
  const tab = `settings-tab-${id}`;
  const state = await browser.execute(() => {
    const rail = document.querySelector('[role="tablist"][aria-label="Settings sections"]');
    const selected = Array.from(rail?.querySelectorAll('[role="tab"][aria-selected="true"]') ?? []);
    const panel = document.getElementById(selected[0]?.getAttribute("aria-controls") ?? "");
    return {
      focused: document.activeElement?.id,
      selected: selected.map((element) => element.id),
      stops: Array.from(rail?.querySelectorAll('[role="tab"][tabindex="0"]') ?? [], (element) => element.id),
      panel: panel?.getAttribute("role"),
      labelledBy: panel?.getAttribute("aria-labelledby"),
    };
  });
  expect(state).toEqual({ focused: tab, selected: [tab], stops: [tab], panel: "tabpanel", labelledBy: tab });
  await expect(browser.$('[role="tabpanel"]')).toBeDisplayed();
}

describe("Settings keyboard tabs", function () {
  this.timeout(180_000);

  it("selects tabs with horizontal keys and leaves the rail with Tab", async () => {
    await createIdentity(browser, newUser("settings-keys"), { keepAutoAway: true });
    await openSettings(browser, "Profile");
    await browser.execute(() => document.getElementById("settings-tab-profile")?.focus());
    await expectTab("profile");
    await browser.keys(Key.ArrowLeft);
    await expectTab("about");
    await expect(browser.$('[role="tabpanel"]')).toHaveText(expect.stringContaining("Check for Updates"));
    await browser.keys(Key.ArrowRight);
    await expectTab("profile");
    await browser.keys(Key.ArrowRight);
    await expectTab("application");
    await expect(browser.$('[role="tabpanel"]')).toHaveText(expect.stringContaining("Start with System"));
    await shot("application-selected-by-keyboard");
    await browser.keys(Key.End);
    await expectTab("about");
    await browser.keys(Key.Home);
    await expectTab("profile");
    await browser.keys(Key.Tab);
    expect(await browser.execute(() => document.activeElement?.id)).toBe("main-content");
    await browser.keys([Key.Shift, Key.Tab]);
    await expectTab("profile");
  });
});
