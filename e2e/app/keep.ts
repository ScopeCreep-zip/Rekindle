// Plan E7.14 S4: one evidence folder per run (e2e/app/out/keep/<run start>/),
// so a later run never overwrites a failure's screenshots or app logs.
// The launcher names it in `onPrepare`, before any worker forks; each worker
// inherits the name through its environment, so the launcher's app logs and
// the workers' failure screenshots land in the same folder.

import { existsSync, readdirSync, rmSync, statSync } from "node:fs";
import path from "node:path";

const RUN_ENV = "REKINDLE_APP_RUN";
const OUT = path.join(import.meta.dirname, "out");

/** How many earlier runs each evidence folder keeps. */
export const KEEP_RUNS = 5;

export function keepDir(): string {
  process.env[RUN_ENV] ??= new Date().toISOString().replace(/[:.]/g, "-");
  return path.join(OUT, "keep", process.env[RUN_ENV]);
}

/** A run folder's name: its start as `toISOString()` with `:` and `.` made `-`, so names sort by time. */
const RUN_NAME = /^\d{4}-\d{2}-\d{2}T\d{2}-\d{2}-\d{2}-\d{3}Z$/;

/** In `dir`, all but the newest `keep` run folders go; anything not named as a run stays. */
export function pruneRuns(dir: string, keep: number): void {
  if (!existsSync(dir)) return;
  const runs = readdirSync(dir)
    .filter((name) => RUN_NAME.test(name) && statSync(path.join(dir, name)).isDirectory())
    .sort();
  for (const name of runs.slice(0, Math.max(0, runs.length - keep))) rmSync(path.join(dir, name), { recursive: true, force: true });
}

/**
 * Cap the evidence runs leave (Linux at ddfc1e70b had 9 runs kept and 2
 * failures accumulating): before a run, each of `out/keep/`,
 * `out/keep/fleet/` and every `out/failed/<spec>/` keeps its newest
 * `KEEP_RUNS` earlier runs.
 */
export function pruneKept(): void {
  pruneRuns(path.join(OUT, "keep"), KEEP_RUNS);
  pruneRuns(path.join(OUT, "keep", "fleet"), KEEP_RUNS);
  const failed = path.join(OUT, "failed");
  if (existsSync(failed)) for (const spec of readdirSync(failed)) pruneRuns(path.join(failed, spec), KEEP_RUNS);
}
