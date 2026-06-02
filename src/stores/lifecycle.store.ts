import { createSignal } from "solid-js";

/**
 * Mirror of the backend `rekindle-lifecycle` FSM (serde `snake_case`).
 * The 9-state path is stopped → starting → locked → resuming → operational
 * (+ degraded / detached / locking / shutting_down).
 */
export type LifecycleState =
  | "stopped"
  | "starting"
  | "locked"
  | "resuming"
  | "operational"
  | "degraded"
  | "detached"
  | "locking"
  | "shutting_down";

/**
 * Pure *derived view* of the single backend authority — never a second
 * lifecycle. Seeded from `lifecycle_current()` and updated by the
 * `lifecycle-event` broadcast (see `subscribeLifecycleEvents`). Mirrors
 * Briar's `StartupViewModel` (seeds `getLifecycleState()` + subscribes
 * `LifecycleEvent`) and Element's `MatrixChat` (`view`/`ready` from
 * `SyncState`).
 */
const [lifecycleState, setLifecycleState] = createSignal<LifecycleState>("starting");

export { lifecycleState, setLifecycleState };

/** Ready to accept an unlock/login — the backend's `can_unlock()` gate. */
export const canUnlock = (): boolean => lifecycleState() === "locked";
