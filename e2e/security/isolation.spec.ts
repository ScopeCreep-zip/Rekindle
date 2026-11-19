import { test, expect, type Page } from "@playwright/test";
import { fileURLToPath } from "node:url";

// Tauri isolation hook (plan E7.1, roadmap 3.3).
//
// `dist-isolation/` is the page Tauri runs in a sandboxed iframe of every
// window; its `__TAURI_ISOLATION_HOOK__` sees each IPC message before Rust
// does. Tauri inlines `commands.js` and `index.js` into that page, so the
// test loads the same two scripts into a blank page and calls the hook with
// the message shape Tauri hands it (`{ cmd, callback, error, payload,
// options }`). A throw is a refusal: Tauri never sends the message.

const file = (name: string): string =>
  fileURLToPath(new URL(`../../dist-isolation/${name}`, import.meta.url));

async function loadHook(page: Page): Promise<void> {
  await page.setContent("<!doctype html><html><body></body></html>");
  await page.addScriptTag({ path: file("commands.js") });
  await page.addScriptTag({ path: file("index.js") });
}

/** The hook's answer: the message it passes, or the refusal's text. */
async function hook(
  page: Page,
  cmd: unknown,
  payload: unknown,
): Promise<{ passed: unknown } | { refused: string }> {
  return page.evaluate(
    async ({ cmd, payload }) => {
      const message = { cmd, callback: 1, error: 2, payload, options: {} };
      const run = (
        window as unknown as {
          __TAURI_ISOLATION_HOOK__: (m: unknown) => unknown;
        }
      ).__TAURI_ISOLATION_HOOK__;
      try {
        return { passed: await run(message) };
      } catch (e) {
        return { refused: String(e) };
      }
    },
    { cmd, payload },
  );
}

test.describe("isolation hook", () => {
  test.beforeEach(async ({ page }) => {
    await loadHook(page);
  });

  test("a registered command with a plain-object payload passes unchanged", async ({
    page,
  }) => {
    const answer = await hook(page, "get_friends", { a: 1 });
    expect(answer).toEqual({
      passed: { cmd: "get_friends", callback: 1, error: 2, payload: { a: 1 }, options: {} },
    });
    expect(await hook(page, "plugin:window|minimize", {})).toHaveProperty("passed");
  });

  test("a command not in the generated list is refused", async ({ page }) => {
    for (const cmd of [
      "not_a_command",
      "plugin:webview|internal_toggle_devtools",
      "plugin:event|emit",
      "plugin:shell|open",
    ]) {
      expect(await hook(page, cmd, {}), cmd).toEqual({
        refused: "Error: isolation: refused an unregistered command",
      });
    }
    expect(await hook(page, 42, {})).toHaveProperty("refused");
  });

  test("a payload that is not a plain object is refused, except the channel fetch's null", async ({
    page,
  }) => {
    expect(await hook(page, "plugin:__TAURI_CHANNEL__|fetch", null)).toHaveProperty("passed");
    for (const payload of [null, [1, 2], "text", 7]) {
      expect(await hook(page, "get_friends", payload), JSON.stringify(payload)).toEqual({
        refused: "Error: isolation: refused a payload that is not a plain object",
      });
    }
    expect(await hook(page, "plugin:__TAURI_CHANNEL__|fetch", [1])).toHaveProperty("refused");
  });
});
