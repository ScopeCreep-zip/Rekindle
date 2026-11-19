import { test, expect, type Page } from "@playwright/test";
import { preloadMocks, clearMocks } from "./fixtures/mocks";
import { openWindow } from "./fixtures/paint";

// Plan E7.7.5: the skin editor, a Settings sub-window, renders the Host's
// report and sends the author's edits. This mock plays the Host: `validate_skin_draft` fails
// the pair `accent.text` on `bg.dark` while the draft sets the accent fill's
// colour as text (4.42:1, plan E7.7.0) and suggests `#3491d8`, the
// Oklab-walk value E7.7.0 recorded; any other draft passes.

const FAILING = "#167bc0";
// `rekindle_skin::draft::NO_AUTHOR`: a draft started from a skin has no
// author of its own yet, and the Host names that as one failing check.
const NO_AUTHOR = "Add your name as the author: the export credits you first.";
const SUGGESTED = "#3491d8";

const HANDLER = `
  const w = window;
  const hex = (v) => "#" + [v.r, v.g, v.b].map((c) => c.toString(16).padStart(2, "0")).join("");
  const accent = (d) => d.tokens["accent.text"] ? hex(d.tokens["accent.text"].data) : "#3491d8";
  const report = (d) => {
    const value = accent(d);
    const fails = value === "${FAILING}";
    return {
      tokens: [
        { id: "accent.text", css: "--color-xfire-accent-text", set: !!d.tokens["accent.text"],
          value: { type: "color", data: d.tokens["accent.text"]?.data ?? { r: 52, g: 145, b: 216, a: 255 } } },
        { id: "bg.dark", css: "--color-xfire-bg-dark", set: false,
          value: { type: "color", data: { r: 10, g: 10, b: 10, a: 255 } } },
      ],
      pairs: [
        { rule: "1.4.3", fg: "accent.text", ground: "bg.dark", ratioX100: fails ? 442 : 583,
          requiredX100: 450, pass: !fails, apcaLcX10: fails ? -322 : -401,
          suggestion: fails ? { token: "accent.text", rgb: [52, 145, 216], ratioX100: 583 } : null },
        { rule: "1.4.3", fg: "text.base", ground: "bg.dark", ratioX100: 1590, requiredX100: 450,
          pass: true, apcaLcX10: -950, suggestion: null },
      ],
      failures: d.author.trim() ? [] : ["${NO_AUTHOR}"],
      license: /^[A-Za-z0-9.+-]+$/.test(d.license) ? { spdx: d.license } : { text: d.license },
      credits: { ...d.credits, authors: [d.author, ...d.credits.authors.filter((a) => a !== d.author)] },
      regionSlots: [{ id: "scrollbar.track", carriesText: false }, { id: "statusbar", carriesText: true }],
      tokensCss: "@layer skin {\\n  :root {\\n    --color-xfire-accent-text: " + value + ";\\n  }\\n}\\n",
      exportable: !fails && !!d.author.trim(),
    };
  };
  switch (cmd) {
    case "list_skins":
      return { skins: [{ id: "builtin:symbiosis", name: "Symbiosis", importedAt: null, credits: null }],
        active: "builtin:symbiosis", login: "builtin:symbiosis", trial: null };
    case "get_skin":
      return { id: "builtin:symbiosis", base: "dark", regions: [], palette: [],
        tokensHref: "skin://localhost/${"a".repeat(64)}/tokens.css" };
    case "skin_draft_start":
      return { name: "Symbiosis", description: "", author: "", license: "CC-BY-4.0",
        credits: { authors: ["Orig Author"], contributors: [], url: null, licence: { text: "CC-BY-4.0" } },
        base: "dark", contrast: "standard", tokens: {}, from: "builtin:symbiosis", regions: {} };
    case "import_skin_region": {
      // The Host's validator on a 16 px image: insets that meet are refused.
      const i = args.fit.data;
      if (args.fit.type === "nineSlice" && (i.top + i.bottom >= 16 || i.left + i.right >= 16)) {
        throw { type: "failed", data: { message: "skin refused: assets/regions/" + args.region
          + ".png: nine-slice insets " + i.top + " " + i.right + " " + i.bottom + " " + i.left
          + " do not fit 16x16" } };
      }
      return { ...args.draft, regions: { ...args.draft.regions,
        [args.region]: { path: "/tmp/track.png", fit: args.fit, textBacking: args.textBacking } } };
    }
    case "export_skin_schema":
      return true;
    case "validate_skin_draft":
      return report(args.draft);
    case "export_skin_draft":
      return "${"b".repeat(64)}";
    default:
      return null;
  }
`;

async function startDraft(page: Page): Promise<void> {
  await preloadMocks(page, "skin-editor", HANDLER);
  await openWindow(page, "/skin-editor");
  await expect(page.getByRole("region", { name: "Skin editor" })).toBeVisible();
}

/** The draft started, its author named, so every check passes. */
async function openEditor(page: Page): Promise<void> {
  await startDraft(page);
  await page.getByLabel("Author").fill("Bea");
  await page.getByLabel("Author").blur();
  await expect(page.locator(".skin-editor-status")).toHaveText("Every check passes.");
}

const exportButton = (page: Page) => page.getByRole("button", { name: "Export…" });

test.describe("Skin editor", () => {
  test.afterEach(async ({ page }) => {
    await clearMocks(page);
  });

  test("Settings opens the editor as its own window, never in place", async ({ page }) => {
    await preloadMocks(page, "settings", HANDLER);
    await openWindow(page, "/settings?tab=appearance");
    await page.getByRole("button", { name: "Edit a skin…" }).click();
    await expect
      .poll(() =>
        page.evaluate(() =>
          (window as any).__ipcCalls.map((c: { cmd: string }) => c.cmd).includes("open_skin_editor_window"),
        ),
      )
      .toBe(true);
    await expect(page.locator(".skin-editor")).toHaveCount(0);
    await expect(page.locator("#skin-draft-tokens")).toHaveCount(0);
    const draftCalls = await page.evaluate(() =>
      (window as any).__ipcCalls
        .map((c: { cmd: string }) => c.cmd)
        .filter((c: string) => c.includes("skin_draft")),
    );
    expect(draftCalls).toEqual([]);
  });

  test("the export carries the author, the licence and the source's credits", async ({ page }) => {
    await openEditor(page);
    const credits = page.getByLabel("Credits the export carries");
    await expect(credits).toContainText("Orig Author");
    await page.getByLabel("Author").fill("Bea");
    await page.getByLabel("Author").blur();
    await page.getByLabel("Licence", { exact: true }).fill("MIT");
    await page.getByLabel("Licence", { exact: true }).blur();
    await expect(credits.locator("li").first()).toHaveText("Bea");
    await expect(credits.locator("li").nth(1)).toHaveText("Orig Author");
    await expect(page.getByText("Read as an SPDX expression.")).toBeVisible();
    await exportButton(page).click();
    await expect(page.locator(".skin-editor-status")).toContainText("Exported bbbbbbbbbbbb");
    const sent = await page.evaluate(() =>
      (window as any).__ipcCalls.filter((c: { cmd: string }) => c.cmd === "export_skin_draft").at(-1)
        .args.draft,
    );
    expect(sent.author).toBe("Bea");
    expect(sent.license).toBe("MIT");
    expect(sent.credits.authors).toEqual(["Orig Author"]);
    expect(sent.credits.licence).toEqual({ text: "CC-BY-4.0" });
    expect(sent.from).toBe("builtin:symbiosis");
  });

  // The Linux walk (2026-11-18): "1 checks fail" with no failing pair, and
  // "Read as an licence text."
  test("a started draft asks for its author, counting checks in words", async ({ page }) => {
    await startDraft(page);
    const status = page.locator(".skin-editor-status");
    await expect(status).toHaveText("1 check fails; export is disabled.");
    await expect(page.locator(".skin-editor-failures li")).toHaveText(NO_AUTHOR);
    await expect(page.locator(".skin-pair-fail")).toHaveCount(0);
    await page.getByLabel("accent.text", { exact: true }).fill(FAILING);
    await expect(status).toHaveText("2 checks fail; export is disabled.");
    await page.getByLabel("Licence", { exact: true }).fill("Free for any use, with credit.");
    await page.getByLabel("Licence", { exact: true }).blur();
    await expect(page.getByText("Read as a licence text.")).toBeVisible();
  });

  test("a region import refused for its nine-slice leaves the draft unchanged", async ({ page }) => {
    await openEditor(page);
    await page.getByLabel("Region", { exact: true }).selectOption("scrollbar.track");
    await page.getByLabel("top").fill("9");
    await page.getByLabel("bottom").fill("9");
    await page.getByRole("button", { name: "Import image…" }).click();
    await expect(page.locator(".skin-error")).toContainText("nine-slice insets 9 4 9 4");
    await expect(page.locator(".skin-regions li")).toHaveCount(0);
    await page.getByLabel("top").fill("4");
    await page.getByLabel("bottom").fill("4");
    await page.getByRole("button", { name: "Import image…" }).click();
    await expect(page.locator(".skin-regions li")).toHaveText("scrollbar.track: nineSlice");
    await page.getByRole("button", { name: "Export JSON Schema…" }).click();
    await expect(page.locator(".skin-editor-status")).toContainText("JSON Schema saved.");
  });

  test("a failing pair disables export and names its ratio", async ({ page }) => {
    await openEditor(page);
    await expect(exportButton(page)).toBeEnabled();
    await page.getByRole("searchbox", { name: "Search keys" }).fill("accent");
    await expect(page.locator(".skin-token")).toHaveCount(1);
    await page.getByLabel("accent.text", { exact: true }).fill(FAILING);
    await expect(exportButton(page)).toBeDisabled();
    const pair = page.locator(".skin-pair-fail");
    await expect(pair).toContainText("accent.text on bg.dark");
    await expect(pair).toContainText("4.42:1 / 4.50:1 Fail");
    await expect(pair).toContainText("APCA Lc -32.2");
  });

  test("clicking the suggestion fixes the pair and enables export", async ({ page }) => {
    await openEditor(page);
    await page.getByLabel("accent.text", { exact: true }).fill(FAILING);
    await expect(exportButton(page)).toBeDisabled();
    await page.getByRole("button", { name: `Use ${SUGGESTED} for accent.text (5.83:1)` }).click();
    await expect(page.locator(".skin-pair-fail")).toHaveCount(0);
    await expect(exportButton(page)).toBeEnabled();
    const sent = await page.evaluate(() =>
      (window as any).__ipcCalls
        .filter((c: { cmd: string }) => c.cmd === "validate_skin_draft")
        .at(-1).args.draft.tokens["accent.text"].data,
    );
    expect(sent).toEqual({ r: 52, g: 145, b: 216, a: 255 });
    await exportButton(page).click();
    await expect(page.locator(".skin-editor-status")).toContainText("Exported bbbbbbbbbbbb");
  });

  test("live apply restyles the editor window without changing the device skin", async ({ page }) => {
    await openEditor(page);
    await page.getByLabel("accent.text", { exact: true }).fill("#ff8800");
    await expect
      .poll(() =>
        page.evaluate(() =>
          getComputedStyle(document.documentElement)
            .getPropertyValue("--color-xfire-accent-text")
            .trim(),
        ),
      )
      .toBe("rgb(255, 136, 0)");
    const global = await page.evaluate(() =>
      (window as any).__ipcCalls
        .map((c: { cmd: string }) => c.cmd)
        .filter((c: string) => ["select_skin", "import_skin", "keep_skin"].includes(c)),
    );
    expect(global).toEqual([]);
  });
});
