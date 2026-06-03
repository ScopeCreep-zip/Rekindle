import { createStore } from "solid-js/store";

/**
 * Transient "dial-in" state for the self-sovereign community join flow.
 *
 * The backend gates each join phase in its own timeout and emits one
 * `joinProgress` community-event per transition (see the Rust
 * `join_gate` module). This store is a pure mirror of that stream so
 * `JoinProgressStepper` can render the live phase list. All timeout /
 * gating logic lives in the backend; the frontend only displays.
 */
export type JoinStageStatus = "started" | "done" | "failed" | "timedOut";

export interface JoinStage {
  label: string;
  status: JoinStageStatus;
}

export interface JoinProgressState {
  /** True from submit until the join promise settles. */
  active: boolean;
  /** Phases in arrival order; each label appears once and is updated in place. */
  stages: JoinStage[];
}

const [joinProgress, setJoinProgress] = createStore<JoinProgressState>({
  active: false,
  stages: [],
});

/** Reset the stepper and mark a join in flight (called on modal submit). */
export function beginJoinProgress(): void {
  setJoinProgress({ active: true, stages: [] });
}

/** Mark the join settled; leaves the final stage statuses visible. */
export function endJoinProgress(): void {
  setJoinProgress("active", false);
}

/** Clear the stepper entirely (e.g. when the join modal re-opens). */
export function resetJoinProgress(): void {
  setJoinProgress({ active: false, stages: [] });
}

/** Apply one backend `joinProgress` event, upserting by phase label. */
export function applyJoinProgress(label: string, status: JoinStageStatus): void {
  setJoinProgress("stages", (prev) => {
    const idx = prev.findIndex((s) => s.label === label);
    if (idx >= 0) {
      const next = prev.slice();
      next[idx] = { label, status };
      return next;
    }
    return [...prev, { label, status }];
  });
}

export { joinProgress, setJoinProgress };
