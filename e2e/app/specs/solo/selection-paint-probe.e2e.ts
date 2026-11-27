// Probe (ST1 blocker): how this WebKit paints an author ::selection colour.
// For known colours, with the page's colour scheme dark and then light, a
// test-only style sets the Profile field's ::selection background, and the
// painted highlight is read from the screenshot. RenderThemeMac::
// transformSelectionBackgroundColor gives an opaque author colour alpha 0.8
// under a dark appearance; this measures whether that, or something more,
// separates the painted colour from the declared one.

import { browser } from "@wdio/globals";
import { openSettings } from "../../../walk/nav.ts";
import { NET_WAIT_MS, shown } from "../../../walk/helpers.ts";
import { createIdentity, newUser, type User } from "../../journeys.ts";
import { decodePng } from "../../png.ts";
import { shot } from "../../shots.ts";

const FIELD = '.settings-content input.form-input[type="text"]';
const COLOURS = ["rgb(255, 0, 0)", "rgb(0, 0, 255)", "rgb(128, 128, 128)", "rgb(58, 58, 58)", "rgb(22, 123, 192)", "rgba(22, 123, 192, 0.5)"];

async function painted(): Promise<{ painted: string; ground: string }> {
  const box = (await browser.execute((s: string) => {
    const f = document.querySelector<HTMLInputElement>(s);
    if (!f) return null;
    const r = f.getBoundingClientRect();
    return { x: r.left + 12, y: r.top + 8, w: r.width - 24, h: r.height - 16, vw: window.innerWidth, ground: getComputedStyle(f).backgroundColor };
  }, FIELD)) as { x: number; y: number; w: number; h: number; vw: number; ground: string } | null;
  if (!box) throw new Error("no field");
  const img = decodePng(Buffer.from(await browser.takeScreenshot(), "base64"));
  const k = img.width / box.vw;
  const counts = new Map<string, number>();
  for (let y = Math.round(box.y * k); y < Math.round((box.y + box.h) * k); y++) {
    for (let x = Math.round(box.x * k); x < Math.round((box.x + box.w) * k); x++) {
      const i = (y * img.width + x) * 4;
      const key = `rgb(${img.data[i]}, ${img.data[i + 1]}, ${img.data[i + 2]})`;
      counts.set(key, (counts.get(key) ?? 0) + 1);
    }
  }
  const ranked = [...counts.entries()].sort((a, b) => b[1] - a[1]).map(([c, n]) => `${c}×${n}`);
  return { painted: ranked.filter((c) => !c.startsWith(`${box.ground}×`)).slice(0, 3).join(" "), ground: box.ground };
}

describe("selection paint probe (ST1)", function () {
  this.timeout(300_000);
  const me: User = { ...newUser("selprobe"), displayName: "Ari" };

  it("declared vs painted ::selection, dark and light scheme", async () => {
    await createIdentity(browser, me);
    await openSettings(browser, "Profile");
    await shown(browser, FIELD, NET_WAIT_MS);
    const env = await browser.execute(() => ({
      scheme: getComputedStyle(document.documentElement).colorScheme,
      dark: matchMedia("(prefers-color-scheme: dark)").matches,
      dpr: devicePixelRatio,
    }));
    console.log(`[selprobe] env ${JSON.stringify(env)}`);
    for (const scheme of ["dark", "light"]) {
      for (const colour of COLOURS) {
        await browser.execute(
          (sel: string, c: string, sc: string) => {
            let st = document.getElementById("selprobe-style");
            if (!st) {
              st = document.createElement("style");
              st.id = "selprobe-style";
              document.head.append(st);
            }
            st.textContent = `:root { color-scheme: ${sc} !important; } ${sel}::selection { background-color: ${c} !important; color: rgb(255, 255, 255) !important; }`;
            const f = document.querySelector<HTMLInputElement>(sel);
            f?.focus();
            f?.select();
          },
          FIELD,
          colour,
          scheme,
        );
        await browser.pause(300);
        const p = await painted();
        console.log(`[selprobe] scheme ${scheme} declared ${colour} → painted ${p.painted} (ground ${p.ground})`);
      }
      await shot(`probe-${scheme}`);
    }
  });
});
