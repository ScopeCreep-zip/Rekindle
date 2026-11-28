// Plan E7.14: the walk keeps the evidence of a failed run, as the real-app
// specs do (e2e/app/launcher.ts). Once any row fails, each app stopped from
// then on (the peer when its rows end, the walk's own app at the end) keeps
// its data root and log in e2e/walk/failed/<run start>/<app>/, and the walk
// says where. The voice rows run while the peer is up, so a failure there
// keeps both sides (VR1, M114: a rejoin not reaching the peer, kept
// nothing). OUT_DIR is wiped at every start, so this lives beside it;
// earlier runs are capped like the specs' (e2e/app/keep.ts).

import path from "node:path";
import { KEEP_RUNS, pruneRuns } from "../app/keep.ts";

const FAILED = path.join(import.meta.dirname, "failed");
const RUN = new Date().toISOString().replace(/[:.]/g, "-");
let failed = false;

/** A row of this run failed: apps stopped from now on keep their roots. */
export function markRowFailed(): void {
  failed = true;
}

/** Where app `name` keeps its root and log when stopped now: this run's folder once a row failed, else nothing. */
export function keepIfFailed(name: string): string | undefined {
  return failed ? path.join(FAILED, RUN, name) : undefined;
}

/** This run's folder, once a row failed. */
export function keptRun(): string | undefined {
  return failed ? path.join(FAILED, RUN) : undefined;
}

/** Before a walk: the newest `KEEP_RUNS` earlier failed runs stay. */
export function pruneFailedRuns(): void {
  pruneRuns(FAILED, KEEP_RUNS);
}
