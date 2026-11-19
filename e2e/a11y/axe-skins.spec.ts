import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { expect, test } from "@playwright/test";
import { FULL } from "../fixtures/stress";
import { WINDOWS, open } from "../layout/windows";
import { violations } from "./axe";

// Plan E7.3 "Gate": axe "over every window under every built-in skin". The
// mocked suite cannot load a `skin://` stylesheet, so each built-in's
// stylesheet is the one the Host serves, written from `rekindle-skin` by
// `cargo xtask gen-skin-fixtures` (`check-skin-fixtures` keeps it current),
// applied the way `useSkin` does: `:root[data-skin-base]` from the pack's
// applied variant and the `@layer skin` sheet. Its images are not served,
// so colours are measured on the grounds the tokens declare.

const dir = (rel: string) =>
  fileURLToPath(new URL(`../fixtures/generated/skins/${rel}`, import.meta.url));

const SKINS: { id: string; base: "dark" | "light" }[] = JSON.parse(
  readFileSync(dir("skins.json"), "utf8"),
);

for (const skin of SKINS) {
  const css = readFileSync(dir(`${skin.id}.css`), "utf8");
  for (const w of WINDOWS) {
    test(`${w.name} builtin:${skin.id}`, async ({ page }) => {
      await open(page, w, w.def, FULL, skin.base);
      await page.addStyleTag({ content: css });
      // Let the restyle land before measuring.
      await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => r(null))));
      expect(await violations(page), `${w.name} (builtin:${skin.id}) axe violations`).toEqual([]);
    });
  }
}
