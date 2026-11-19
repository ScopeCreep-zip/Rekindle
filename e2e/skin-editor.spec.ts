import { test, expect, type Page } from "@playwright/test";
import { preloadMocks, clearMocks } from "./fixtures/mocks";

// Plan E7.7.5: the skin editor renders the Host's report and sends the
// author's edits. This mock plays the Host: `validate_skin_draft` fails
// the pair `accent.text` on `bg.dark` while the draft sets the accent fill's
// colour as text (4.42:1, plan E7.7.0) and suggests `#3491d8`, the
// Oklab-walk value E7.7.0 recorded; any other draft passes.

const FAILING = "#167bc0";
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
      failures: [],
      tokensCss: "@layer skin {\\n  :root {\\n    --color-xfire-accent-text: " + value + ";\\n  }\\n}\\n",
      exportable: !fails,
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
      return { name: "Symbiosis", base: "dark", contrast: "standard", tokens: {} };
    case "validate_skin_draft":
      return report(args.draft);
    case "export_skin_draft":
      return "${"b".repeat(64)}";
    default:
      return null;
  }
`;

async function openEditor(page: Page): Promise<void> {
  await preloadMocks(page, "settings", HANDLER);
  await page.goto("/settings?tab=appearance");
  await page.getByRole("button", { name: "Edit a skin…" }).click();
  await expect(page.getByRole("region", { name: "Skin editor" })).toBeVisible();
  await expect(page.locator(".skin-editor-status")).toHaveText("Every check passes.");
}

const exportButton = (page: Page) => page.getByRole("button", { name: "Export…" });

test.describe("Skin editor", () => {
  test.afterEach(async ({ page }) => {
    await clearMocks(page);
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

  test("live apply restyles only this window and goes with the editor", async ({ page }) => {
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
    await page.getByRole("button", { name: "Close the editor" }).click();
    await expect(page.locator("#skin-draft-tokens")).toHaveCount(0);
  });
});
