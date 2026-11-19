import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { preloadMocks } from "./fixtures/mocks";
import { FULL, PEER, stressHandler } from "./fixtures/stress";
import { violations } from "./a11y/axe";

// Plan E7.3, wiring guide §13f: a "Keyboard shortcuts" help dialog lists
// the shortcuts the app has, opens with Mod+/ in any window and from the
// buddy list's action bar, and is a modal dialog like the others (§13c).

/** The OS-wide chords `src-tauri/src/shortcuts.rs` registers, per platform. */
function globalChords(mac: boolean): string[] {
  const source = readFileSync(
    fileURLToPath(new URL("../src-tauri/src/shortcuts.rs", import.meta.url)),
    "utf8",
  );
  const lines = source.split("\n");
  const cfg = mac ? '#[cfg(target_os = "macos")]' : '#[cfg(not(target_os = "macos"))]';
  const at = lines.findIndex((l) => l.trim() === cfg);
  expect(at, `shortcuts.rs has ${cfg}`).toBeGreaterThanOrEqual(0);
  const chords = [...lines[at + 1].matchAll(/"([^"]+)"/g)].map((m) => m[1]);
  expect(chords.length).toBeGreaterThan(0);
  const label: Record<string, string> = { super: "Cmd", ctrl: "Ctrl", shift: "Shift", alt: "Alt" };
  return chords.map((c) =>
    c.split("+").map((k) => label[k] ?? k.toUpperCase()).join("+"),
  );
}

test("the action bar opens the shortcuts, which list the global chords; Esc returns focus", async ({ page }) => {
  await preloadMocks(page, "buddy-list", stressHandler(FULL));
  await page.goto("/buddy-list");
  const opener = page.getByRole("button", { name: "Keyboard shortcuts" });
  await opener.focus();
  await page.keyboard.press("Enter");
  const dialog = page.getByRole("dialog", { name: "Keyboard shortcuts" });
  await expect(dialog).toBeVisible();

  const mac = await page.evaluate(() => /Mac/i.test(navigator.platform));
  const keys = await dialog.locator(".shortcuts-help-chord").allTextContents();
  for (const chord of globalChords(mac)) {
    expect(keys, `the dialog lists ${chord}`).toContain(chord);
  }
  await expect(dialog.getByRole("region", { name: "Anywhere" })).toContainText("Show or hide the buddy list");
  await expect(dialog.getByRole("region", { name: "Lists" })).toContainText("Shift+F10");
  expect(await violations(page)).toEqual([]);

  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(opener).toBeFocused();
});

test("Mod+/ opens the shortcuts in a chat window", async ({ page }) => {
  await preloadMocks(page, `chat-${PEER.slice(0, 16)}`, stressHandler(FULL));
  await page.goto(`/chat?peer=${PEER}`);
  const input = page.getByRole("combobox", { name: /^Message / });
  await input.focus();
  await page.keyboard.press("ControlOrMeta+/");
  const dialog = page.getByRole("dialog", { name: "Keyboard shortcuts" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole("region", { name: "Chat" })).toContainText("Send the message");
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(input).toBeFocused();
});
