// The Host's own trace, read from the log of the app a spec drives. The
// launcher pipes each app's stdout and stderr (the desktop's tracing) into
// e2e/walk/out/wdio-<cid>.log, or wdio-<cid>-<instance>.log for a
// multiremote instance (walk/launch.ts): one file per app, never
// interleaved. Specs count existing scrubbed trace lines; nothing is added
// to the Host's logs for them.

import { readdirSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import { OUT_DIR } from "../walk/launch.ts";

/** This run's log of the app launched alone, or as multiremote `instance`: the newest file of that name. */
export function appLogPath(instance?: string): string {
  const name = instance ? new RegExp(`^wdio-\\d+-\\d+-${instance}\\.log$`) : /^wdio-\d+-\d+\.log$/;
  const files = readdirSync(OUT_DIR)
    .filter((f) => name.test(f))
    .map((f) => path.join(OUT_DIR, f));
  if (files.length === 0) throw new Error(`no app log for ${instance ?? "the app"} in ${OUT_DIR}`);
  return files.sort((a, b) => statSync(b).mtimeMs - statSync(a).mtimeMs)[0]!;
}

/** How many times `line` appears in the log at `file` so far. */
export function countInLog(file: string, line: string): number {
  return readFileSync(file, "utf8").split(line).length - 1;
}

/** The Host's presence reconcile re-sending our voice join to members missing from our roster (rekindle-voice roster_reconcile). */
export const RECONCILE_RESENT = "presence reconcile: re-sent our voice join to members missing from our roster";
