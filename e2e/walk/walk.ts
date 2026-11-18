// Plan E7.13: the whole-app walk. Builds the desktop app with its embedded
// WebDriver server (unless --no-build), launches it on a fresh data root,
// walks the rows in rows.ts with one throwaway identity, and writes a
// screenshot per row, e2e/walk/out/report.json and a printed table.
//
//   pnpm walk              build, then walk
//   pnpm walk --no-build   walk the last debug build

import { randomBytes } from "node:crypto";
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import path from "node:path";
import type { Browser } from "webdriverio";
import { Skip, settleAnimations } from "./helpers.ts";
import { OUT_DIR, build, launch, openSession, renewSession, type App } from "./launch.ts";
import { allRows, type RowContext, type WalkState } from "./rows.ts";
import { renewPeerSession, stopPeer } from "./rows-peer.ts";

type Status = "pass" | "fail" | "skip";

/** A row past `ROW_TIMEOUT_MS`; its sessions are renewed so it stops driving the app. */
class RowTimeout extends Error {}

interface Result {
  row: string;
  title: string;
  status: Status;
  note: string;
  screenshot: string | null;
  ms: number;
}

function message(err: unknown): string {
  const text = err instanceof Error ? err.message : String(err);
  return text.split("\n")[0].slice(0, 300);
}

async function screenshot(b: Browser, row: string): Promise<string | null> {
  const file = path.join(OUT_DIR, `${row}.png`);
  try {
    await settleAnimations(b);
    await b.saveScreenshot(file);
    return path.relative(OUT_DIR, file);
  } catch {
    return null;
  }
}

async function walk(app: App): Promise<Result[]> {
  let b = await openSession(app, "login");

  const s: WalkState = {
    displayName: `walk-${randomBytes(3).toString("hex")}`,
    passphrase: randomBytes(18).toString("base64url"),
    communityName: `walk-community-${randomBytes(2).toString("hex")}`,
  };
  const results: Result[] = [];
  const ROW_TIMEOUT_MS = 300_000;
  const SHOT_TIMEOUT_MS = 15_000;
  try {
    for (const row of await allRows()) {
      const started = Date.now();
      let shotPath: string | null = null;
      let taken = false;
      const rb = b;
      const ctx: RowContext = {
        b: rb,
        app,
        s,
        async shot() {
          shotPath = await screenshot(rb, row.id);
          taken = true;
        },
      };
      let status: Status = "pass";
      let note = "";
      try {
        note = (await withDeadline(row.run(ctx), ROW_TIMEOUT_MS, new RowTimeout(`row timed out after ${ROW_TIMEOUT_MS} ms`))) ?? "";
      } catch (err) {
        status = err instanceof Skip ? "skip" : "fail";
        note = message(err);
        // A row past its deadline is still running: end the sessions it
        // holds so its next command fails instead of driving the app while
        // the next row runs (B9's failure in walk 2).
        if (err instanceof RowTimeout) {
          if (app.alive()) b = await renewSession(app, b);
          await renewPeerSession();
        }
      }
      // A row that closed the window the session points at (B64's logout
      // closes the buddy list) leaves every later command failing with "No
      // window could be found": point the session at a live window first.
      if (app.alive()) await reattach(b);
      if (!taken && app.alive()) {
        shotPath = await withDeadline(screenshot(b, row.id), SHOT_TIMEOUT_MS, new Error("screenshot timed out")).catch(() => null);
      }
      results.push({ row: row.id, title: row.title, status, note, screenshot: shotPath, ms: Date.now() - started });
      console.log(`[walk] ${row.id} ${status}${note ? `: ${note}` : ""}`);
    }
  } finally {
    await stopPeer();
    if (app.alive()) await b.deleteSession().catch(() => undefined);
  }
  return results;
}

function printTable(results: Result[]): void {
  const w = Math.max(3, ...results.map((r) => r.row.length));
  console.log(`\n${"row".padEnd(w)} | pass/fail | note`);
  console.log(`${"-".repeat(w)}-|-----------|-----`);
  for (const r of results) console.log(`${r.row.padEnd(w)} | ${r.status.padEnd(9)} | ${r.note || r.title}`);
  const count = (st: Status): number => results.filter((r) => r.status === st).length;
  console.log(`\n${count("pass")} pass, ${count("fail")} fail, ${count("skip")} skip of ${results.length}`);
}

async function main(): Promise<number> {
  rmSync(OUT_DIR, { recursive: true, force: true });
  mkdirSync(OUT_DIR, { recursive: true });
  if (!process.argv.includes("--no-build")) build();
  const app = await launch();
  let results: Result[] = [];
  try {
    results = await walk(app);
  } finally {
    await app.stop();
  }
  writeFileSync(
    path.join(OUT_DIR, "report.json"),
    JSON.stringify({ date: new Date().toISOString(), platform: process.platform, results }, null, 2),
  );
  printTable(results);
  return results.some((r) => r.status === "fail") ? 1 : 0;
}

main().then(
  (code) => process.exit(code),
  (err: unknown) => {
    console.error(`[walk] aborted: ${message(err)}`);
    process.exit(2);
  },
);

/** `work`, or a rejection with `err` after `ms`: a stuck row fails instead of hanging the walk. */
function withDeadline<T>(work: Promise<T>, ms: number, err: Error): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const deadline = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(err), ms);
  });
  return Promise.race([work, deadline]).finally(() => clearTimeout(timer));
}

/** Switch to a live window when the current one is gone (buddy list first, then login). */
async function reattach(b: Browser): Promise<void> {
  const handles = await b.getWindowHandles().catch(() => [] as string[]);
  const current = await b.getWindowHandle().catch(() => "");
  if (handles.includes(current)) return;
  const next = handles.find((h) => h === "buddy-list") ?? handles.find((h) => h === "login") ?? handles[0];
  if (next) await b.switchToWindow(next).catch(() => undefined);
}
