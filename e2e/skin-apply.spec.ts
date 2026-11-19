import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { preloadMocks, clearMocks } from "./fixtures/mocks";
import { openWindow } from "./fixtures/paint";

// Plan E7.7.2 "Apply": every window asks the Host for the skin it shows
// (`get_skin`), sets `:root[data-skin-base]` from its active variant and
// points one `<link rel="stylesheet">` at its `skin://` stylesheet; a
// `SkinChanged` event on `skin-event` swaps both. The login window shows
// the device login skin the same way.

// The event is the golden sample `rekindle-types` serializes from the Rust
// `SystemEvent::SkinChanged`, not a hand-written payload.
const GOLDEN = JSON.parse(
  readFileSync(
    fileURLToPath(
      new URL("../crates/rekindle-types/tests/golden/subscription_events.json", import.meta.url),
    ),
    "utf8",
  ),
) as Record<string, { system: { skinChanged: { skin: { base: string; tokensHref: string } } } }>;
const CHANGED = GOLDEN["system.skinChanged"];

const A = "a".repeat(64);

const HANDLER = `
  switch (cmd) {
    case "subscribe_events":
      window.__eventChannelId = args.channel.id;
      return null;
    case "get_skin":
      return {
        id: "builtin:symbiosis",
        base: "light",
        tokensHref: "skin://localhost/${A}/tokens.css",
        regions: ["frame"],
      };
    case "list_identities":
      return [{ publicKey: "abc123def456", displayName: "TestUser", createdAt: 1000 }];
    default:
      return null;
  }
`;

test.describe("Skin apply", () => {
  test.afterEach(async ({ page }) => {
    await clearMocks(page);
  });

  test("the login window applies the skin and follows SkinChanged", async ({ page }) => {
    await preloadMocks(page, "login", HANDLER);
    await openWindow(page, "/login");
    await page.waitForSelector(".login-title", { timeout: 10_000 });

    const root = page.locator("html");
    const link = page.locator('link#rekindle-skin[rel="stylesheet"]');
    await expect(root).toHaveAttribute("data-skin-base", "light");
    await expect(link).toHaveAttribute("href", `skin://localhost/${A}/tokens.css`);
    expect(
      await page.evaluate(() =>
        (window as any).__ipcCalls.some((c: { cmd: string }) => c.cmd === "get_skin"),
      ),
    ).toBe(true);

    // The backend announces another skin on the window's stream.
    await page.waitForFunction(() => (window as any).__eventChannelId !== undefined);
    await page.evaluate(
      ({ payload }) => {
        const w = window as any;
        w.__TAURI_INTERNALS__.runCallback(w.__eventChannelId, {
          index: 0,
          message: { cursor: { epoch: 1, seq: 1 }, channel: "skin-event", payload },
        });
      },
      { payload: CHANGED },
    );
    const skin = CHANGED.system.skinChanged.skin;
    await expect(root).toHaveAttribute("data-skin-base", skin.base);
    await expect(link).toHaveAttribute("href", skin.tokensHref);
    await expect(page.locator("link#rekindle-skin")).toHaveCount(1);
  });
});
