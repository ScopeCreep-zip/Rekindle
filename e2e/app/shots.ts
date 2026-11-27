// Every real-app test is validated by a screenshot of the state it asserts
// (owner rule, 2026-11-23): a green assertion alone is no validation. After
// each test, passed or failed, every window each user's app has open is saved
// to e2e/app/out/shots/<spec>/<test>/<user>-<window>.png (base.conf.ts
// afterTest); a spec asserting an intermediate state (a dialog open, a toast
// shown) calls `shot(name)` there. Each folder's index.json names every shot's
// test, window, skin sheet and the build's commit. e2e/app/out is git-ignored.

import { execSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { browser } from "@wdio/globals";
import type { Browser } from "webdriverio";

const SHOTS = path.join(import.meta.dirname, "out", "shots");

/** The checked-out commit the app was built from, once per run. */
const SHA = (() => {
  try {
    return execSync("git rev-parse --short HEAD", { cwd: import.meta.dirname }).toString().trim();
  } catch {
    return "unknown";
  }
})();

/**
 * The test the next shots belong to (set by base.conf.ts beforeTest), on
 * globalThis: the config and the specs load this module as separate
 * instances, so a module variable set from the config was never seen by a
 * spec's shot() (its shots went to spec/test).
 */
interface Current {
  spec: string;
  test: string;
}
const KEY = "__rekindleShotsCurrent";
const holder = globalThis as unknown as Record<string, Current | undefined>;
const current = (): Current => holder[KEY] ?? { spec: "spec", test: "test" };

const slug = (s: string): string => s.replace(/[^\w-]+/g, "_").replace(/^_+|_+$/g, "").slice(0, 80) || "_";

/** Name the test the following shots belong to: its spec file and its full title. */
export function shotsFor(file: string, parent: string, title: string): void {
  // The test's own title leads (the describe title is shared by every test
  // and filled the 80 characters, so two tests of one spec shared a folder
  // and the second's shots replaced the first's); a short hash of the full
  // title keeps two long titles apart.
  const full = parent ? `${parent} ${title}` : title;
  const tag = createHash("sha256").update(full).digest("hex").slice(0, 6);
  holder[KEY] = { spec: slug(path.basename(file).replace(/\.e2e\.ts$/, "")), test: `${slug(title).slice(0, 72)}-${tag}` };
}

/** Every user's session: one standalone, or each multiremote instance by name. */
function sessions(): [string, Browser][] {
  if (!browser.isMultiRemote) return [["app", browser as unknown as Browser]];
  const multi = browser as unknown as WebdriverIO.MultiRemoteBrowser;
  return multi.instances.map((name) => [name, multi.getInstance(name)]);
}

interface ShotEntry {
  file: string;
  test: string;
  user: string;
  window: string;
  skin: string;
  sha: string;
}

/** Save session `b`'s current window as `file` in the test's folder, and index it. */
async function save(b: Browser, user: string, file: string): Promise<void> {
  const now = current();
  const dir = path.join(SHOTS, now.spec, now.test);
  mkdirSync(dir, { recursive: true });
  const window = await b.getWindowHandle().catch(() => "none");
  // The window's linked skin sheet (useSkin.ts LINK_ID): which skin it shows.
  const skin = ((await b
    .execute(() => document.getElementById("rekindle-skin")?.getAttribute("href") ?? "")
    .catch(() => "")) as string) || "none";
  await b.saveScreenshot(path.join(dir, file));
  const index = path.join(dir, "index.json");
  const entries: ShotEntry[] = existsSync(index) ? (JSON.parse(readFileSync(index, "utf8")) as ShotEntry[]) : [];
  entries.push({ file, test: now.test, user, window, skin, sha: SHA });
  writeFileSync(index, `${JSON.stringify(entries, null, 2)}\n`);
}

/** Whether session `b`'s app is still running: false only when its driver is
 *  gone (the app quit, as a spec's quit step does); any other failure throws. */
async function running(b: Browser): Promise<boolean> {
  return b.getWindowHandles().then(
    () => true,
    (e: unknown) => {
      if (/ECONNREFUSED|ECONNRESET|invalid session id|session deleted/i.test(String(e))) return false;
      throw e;
    },
  );
}

/** The state a spec asserts mid-test (a dialog open, a toast shown): each running user's current window,
 *  as `<user>-<name>.png`; a user whose app has quit has no window to shoot. */
export async function shot(name: string): Promise<void> {
  for (const [user, b] of sessions()) {
    if (!(await running(b))) {
      console.log(`[shot] ${user}: the app has quit, no "${name}" shot`);
      continue;
    }
    await save(b, user, `${user}-${slug(name)}.png`);
  }
}

/**
 * Every window each user's app has open, as `<user>-<window>.png`, then back to
 * the window the test left current (the next test starts where this one ended).
 */
export async function shotAllWindows(): Promise<void> {
  for (const [user, b] of sessions()) {
    const handles = await b.getWindowHandles().catch(() => [] as string[]);
    const was = await b.getWindowHandle().catch(() => "");
    for (const h of handles) {
      try {
        await b.switchToWindow(h);
        await save(b, user, `${user}-${slug(h)}.png`);
      } catch (e) {
        console.log(`[shots] ${user}: window ${h} not saved: ${e instanceof Error ? e.message.split("\n")[0] : String(e)}`);
      }
    }
    if (was && handles.includes(was)) await b.switchToWindow(was).catch(() => undefined);
  }
}
