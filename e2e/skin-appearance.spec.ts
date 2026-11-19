import { test, expect, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { preloadMocks, clearMocks } from "./fixtures/mocks";
import { openWindow } from "./fixtures/paint";

// Plan E7.7.2: Settings → Appearance lists the Host's skins and sends the
// user's choices; choosing a library skin puts it on trial, and the tab
// renders "Keep this skin?" from `SkinTrialChanged` only. The revert is the
// Host's: this mock plays the Host, announcing the trial's end 15 s after
// the select unless `keep_skin` arrives first.

// The trial event is the golden sample `rekindle-types` serializes from
// the Rust `SystemEvent::SkinTrialChanged`.
type TrialPayload = {
  system: { skinTrialChanged: { trial: { id: string; name: string; revertsAtMs: number } | null } };
};
const GOLDEN = JSON.parse(
  readFileSync(
    fileURLToPath(
      new URL("../crates/rekindle-types/tests/golden/subscription_events.json", import.meta.url),
    ),
    "utf8",
  ),
) as Record<string, TrialPayload>;
// The credits are the golden `SkinEntry` samples (plan E7.7.3d).
const DTO = JSON.parse(
  readFileSync(
    fileURLToPath(new URL("../crates/rekindle-types/tests/golden/dto.json", import.meta.url)),
    "utf8",
  ),
) as Record<string, { credits: unknown }>;
const BUILTIN_CREDITS = DTO["skin.entry.builtin"].credits;
const LIBRARY_CREDITS = DTO["skin.entry.library"].credits;
const TRIAL_EVENT = GOLDEN["system.skinTrialChanged"];
const TRIAL = TRIAL_EVENT.system.skinTrialChanged.trial!;
const STARTED = TRIAL.revertsAtMs - 15_000;

const HANDLER = `
  const w = window;
  const trial = ${JSON.stringify(TRIAL)};
  w.__skin = w.__skin ?? { active: "builtin:symbiosis", trial: null };
  w.__evIndex = w.__evIndex ?? 0;
  const library = () => ({
    skins: [
      { id: "builtin:symbiosis", name: "Symbiosis", importedAt: null,
        credits: ${JSON.stringify(BUILTIN_CREDITS)} },
      { id: "builtin:default", name: "Default", importedAt: null, credits: null },
      { id: trial.id, name: trial.name, importedAt: 1000,
        credits: ${JSON.stringify(LIBRARY_CREDITS)} },
    ],
    active: w.__skin.active,
    login: "builtin:symbiosis",
    trial: w.__skin.trial,
  });
  // A Tauri channel delivers by index, so each message takes the next one.
  const emitTrial = (t) => w.__TAURI_INTERNALS__.runCallback(w.__eventChannelId, {
    index: w.__evIndex++,
    message: { cursor: null, channel: "skin-trial-event",
      payload: { system: { skinTrialChanged: { trial: t } } } },
  });
  switch (cmd) {
    case "subscribe_events":
      w.__eventChannelId = args.channel.id;
      return null;
    case "get_skin":
      return { id: w.__skin.trial ? w.__skin.trial.id : w.__skin.active, base: "dark", regions: [],
        tokensHref: "skin://localhost/${"a".repeat(64)}/tokens.css" };
    case "list_skins":
      return library();
    case "select_skin":
      if (args.id.startsWith("pkg:")) {
        w.__skin.trial = trial;
        w.__skinTimer = setTimeout(() => { w.__skin.trial = null; emitTrial(null); }, 15000);
      } else {
        w.__skin.active = args.id;
      }
      return library();
    case "keep_skin":
      clearTimeout(w.__skinTimer);
      if (args.keep) w.__skin.active = args.id;
      w.__skin.trial = null;
      setTimeout(() => emitTrial(null), 0);
      return library();
    default:
      return null;
  }
`;

/**
 * `paused` stops the page clock at the trial's start, so a countdown is
 * read at known instants (only `clock.runFor` moves it) instead of racing
 * the wall clock under load.
 */
async function openAppearance(page: Page, opts: { paused?: boolean } = {}): Promise<void> {
  if (opts.paused) {
    await page.clock.install({ time: STARTED - 60_000 });
    await page.clock.pauseAt(STARTED);
  } else {
    await page.clock.install({ time: STARTED });
  }
  await preloadMocks(page, "settings", HANDLER);
  await openWindow(page, "/settings?tab=appearance");
  await expect(page.getByRole("radio", { name: TRIAL.name })).toBeVisible({ timeout: 10_000 });
  await page.waitForFunction(() => (window as any).__eventChannelId !== undefined);
}

/** Choose the library skin and deliver the Host's trial announcement. */
async function chooseLibrarySkin(page: Page): Promise<void> {
  await page.getByRole("radio", { name: TRIAL.name }).check();
  await page.evaluate(
    ({ payload }) => {
      const w = window as any;
      w.__TAURI_INTERNALS__.runCallback(w.__eventChannelId, {
        index: w.__evIndex++,
        message: { cursor: { epoch: 1, seq: 1 }, channel: "skin-trial-event", payload },
      });
    },
    { payload: TRIAL_EVENT },
  );
}

test.describe("Settings → Appearance", () => {
  test.afterEach(async ({ page }) => {
    await clearMocks(page);
  });

  test("lists the skins and marks the one shown and the login skin", async ({ page }) => {
    await openAppearance(page);
    await expect(page.getByRole("radio", { name: "Symbiosis" })).toBeChecked();
    await expect(page.locator(".skin-badge")).toHaveText("Login skin");
    await expect(page.getByRole("button", { name: "Remove" })).toHaveCount(1);
  });

  test("each skin's info button opens its credits", async ({ page }) => {
    await openAppearance(page);
    const info = page.getByRole("button", { name: "Credits for Symbiosis" });
    await expect(info).toHaveAttribute("aria-expanded", "false");
    await info.focus();
    await page.keyboard.press("Enter");
    await expect(info).toHaveAttribute("aria-expanded", "true");
    const panel = page.getByRole("region", { name: "Credits for Symbiosis" });
    await expect(panel).toContainText("Original authors");
    await expect(panel).toContainText("Xfire, Inc.");
    await expect(panel).toContainText("Tim Brengle");
    await expect(panel).toContainText("http://www.xfire.com/");
    await expect(panel).toContainText("No licence stated");
    await expect(panel).not.toContainText("As stated by the skin");
    await panel.getByRole("button", { name: "Open the source of Symbiosis" }).click();
    const open = await page.evaluate(() =>
      (window as any).__ipcCalls.find((c: { cmd: string }) => c.cmd === "open_external_url"),
    );
    expect(open.args).toEqual({ url: "http://www.xfire.com/" });

    await page.getByRole("button", { name: `Credits for ${TRIAL.name}` }).click();
    await expect(panel).toHaveCount(0);
    const library = page.getByRole("region", { name: `Credits for ${TRIAL.name}` });
    await expect(library).toContainText("As stated by the skin (not verified)");
    await expect(library).toContainText("Free to share.");
    await expect(library.getByRole("button", { name: /Open the source/ })).toHaveCount(0);

    await page.getByRole("button", { name: "Credits for Default" }).click();
    await expect(page.getByRole("region", { name: "Credits for Default" })).toContainText(
      "None stated",
    );
  });

  test("a library skin asks to be kept and goes back after 15 s", async ({ page }) => {
    await openAppearance(page, { paused: true });
    await chooseLibrarySkin(page);
    const prompt = page.locator(".skin-trial");
    await expect(prompt).toContainText("Keep this skin?");
    await expect(prompt).toContainText(`${TRIAL.name} goes back to the previous skin in 15 s`);
    await page.clock.runFor(5_000);
    await expect(prompt).toContainText("in 10 s");
    await page.clock.runFor(9_000);
    await expect(prompt).toContainText("in 1 s");
    await page.clock.runFor(1_000);
    await expect(prompt).toHaveCount(0);
    await expect(page.getByRole("radio", { name: "Symbiosis" })).toBeChecked();
    const calls = await page.evaluate(() =>
      (window as any).__ipcCalls.map((c: { cmd: string }) => c.cmd),
    );
    expect(calls).not.toContain("keep_skin");
  });

  test("keeping the skin sends keep_skin and closes the prompt", async ({ page }) => {
    await openAppearance(page);
    await chooseLibrarySkin(page);
    await page.getByRole("button", { name: "Keep this skin" }).click();
    await expect(page.locator(".skin-trial")).toHaveCount(0);
    const keep = await page.evaluate(() =>
      (window as any).__ipcCalls.find((c: { cmd: string }) => c.cmd === "keep_skin"),
    );
    expect(keep.args).toEqual({ id: TRIAL.id, keep: true });
    await page.clock.runFor(20_000);
    await expect(page.getByRole("radio", { name: TRIAL.name })).toBeChecked();
  });
});
