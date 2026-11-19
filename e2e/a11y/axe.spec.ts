import { expect, test } from "@playwright/test";
import { FULL } from "../fixtures/stress";
import { WINDOWS, open } from "../layout/windows";
import { violations } from "./axe";

// Plan E7.3 "Gate": axe-core over every routed window, at its default size
// over the layout suite's stress data, under the token skin's dark and light
// bases. A violation fails the window's test with its rule id, impact and
// the offending nodes.

for (const base of ["dark", "light"] as const) {
  for (const w of WINDOWS) {
    test(`${w.name} ${base}`, async ({ page }) => {
      await open(page, w, w.def, FULL, base);
      expect(await violations(page), `${w.name} (${base}) axe violations`).toEqual([]);
    });
  }
}
