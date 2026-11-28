// Plan E7.14 S5 (moved from E7.12's e2e/layout/select-contrast.spec.ts):
// WebKitGTK drew the native <select> as a light Adwaita box under a dark
// skin, its text unreadable (E7.13 Linux finding). Every select is
// skin-drawn now (theme/91-top-level.css: `appearance: none`,
// `color-scheme` from the skin's base, themed options). Under every
// built-in skin, applied for real, each select in Settings → Application
// and each of its options must be drawn by the skin with the skin's base as
// its colour scheme, on an opaque ground, its text at WCAG 1.4.3's 4.5:1.

import { browser, expect } from "@wdio/globals";
import { byText, shown } from "../../../walk/helpers.ts";
import { openSettings } from "../../../walk/nav.ts";
import { applySkin, createIdentity, newUser, type User } from "../../journeys.ts";
import { builtinPacks } from "../../skin-pack.ts";

type Rgba = [number, number, number, number];

/** A computed colour (`rgb()`, `rgba()` or `color(srgb …)`), channels 0-1. */
function parse(css: string): Rgba {
  const srgb = css.match(/^color\(srgb ([\d.e-]+) ([\d.e-]+) ([\d.e-]+)(?: \/ ([\d.e-]+))?\)$/);
  if (srgb) return [Number(srgb[1]), Number(srgb[2]), Number(srgb[3]), srgb[4] ? Number(srgb[4]) : 1];
  const rgb = css.match(/^rgba?\(([\d.]+), ([\d.]+), ([\d.]+)(?:, ([\d.]+))?\)$/);
  if (rgb) return [Number(rgb[1]) / 255, Number(rgb[2]) / 255, Number(rgb[3]) / 255, rgb[4] ? Number(rgb[4]) : 1];
  throw new Error(`unparsed colour ${css}`);
}

/** WCAG 2.2 relative luminance (§ relative luminance definition). */
function luminance([r, g, b]: Rgba): number {
  const lin = (c: number): number => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

/** WCAG 2.2 contrast ratio. */
function contrast(fg: Rgba, bg: Rgba): number {
  const [a, b] = [luminance(fg), luminance(bg)].sort((x, y) => y - x);
  return (a + 0.05) / (b + 0.05);
}

interface Painted {
  what: string;
  color: string;
  background: string;
  appearance: string;
  scheme: string;
}

/** Every select in the window and each of its options, as the engine paints them. */
const PAINTED = `var out = [];
var selects = document.querySelectorAll("select.form-select");
for (var i = 0; i < selects.length; i++) {
  var s = selects[i], cs = getComputedStyle(s);
  var label = (s.getAttribute("aria-label") || s.id || ("select " + i));
  out.push({ what: label, color: cs.color, background: cs.backgroundColor, appearance: cs.appearance, scheme: cs.colorScheme });
  for (var j = 0; j < s.options.length; j++) {
    var o = s.options[j], os = getComputedStyle(o);
    out.push({ what: label + " option " + JSON.stringify(o.textContent.trim()), color: os.color, background: os.backgroundColor, appearance: "none", scheme: cs.colorScheme });
  }
}
return { base: document.documentElement.dataset.skinBase || "", painted: out };`;

describe("every select reads under every built-in skin", function () {
  this.timeout(900_000);
  const me: User = newUser("select");

  before(async () => {
    await createIdentity(browser, me);
    await openSettings(browser, "Appearance");
  });

  for (const pack of builtinPacks()) {
    it(`${pack.name}: Settings → Application's selects and options are skin-drawn, opaque and at 4.5:1`, async () => {
      await applySkin(browser, me, pack.name);
      await (await byText(browser, ".form-tab", "Application")).click();
      await shown(browser, "select.form-select");
      const { base, painted } = (await browser.execute(PAINTED)) as { base: string; painted: Painted[] };
      expect(base === "dark" || base === "light").toBe(true);
      const selects = painted.filter((p) => !p.what.includes(" option "));
      expect(selects.length).toBeGreaterThan(0);
      const wrong: string[] = [];
      for (const p of painted) {
        if (p.appearance !== "none") wrong.push(`${p.what}: appearance ${p.appearance}, not skin-drawn`);
        if (p.scheme !== base) wrong.push(`${p.what}: color-scheme ${p.scheme}, the skin's base is ${base}`);
        const bg = parse(p.background);
        if (bg[3] !== 1) wrong.push(`${p.what}: ground ${p.background} is not opaque`);
        const ratio = contrast(parse(p.color), bg);
        if (ratio < 4.5) wrong.push(`${p.what}: ${p.color} on ${p.background} is ${ratio.toFixed(2)}:1`);
      }
      expect(wrong).toEqual([]);
    });
  }
});
