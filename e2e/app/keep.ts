// Plan E7.14 S4: one evidence folder per run (e2e/app/out/keep/<run start>/),
// so a later run never overwrites a failure's screenshots or app logs.
// The launcher names it in `onPrepare`, before any worker forks; each worker
// inherits the name through its environment, so the launcher's app logs and
// the workers' failure screenshots land in the same folder.

import path from "node:path";

const RUN_ENV = "REKINDLE_APP_RUN";

export function keepDir(): string {
  process.env[RUN_ENV] ??= new Date().toISOString().replace(/[:.]/g, "-");
  return path.join(import.meta.dirname, "out", "keep", process.env[RUN_ENV]);
}
