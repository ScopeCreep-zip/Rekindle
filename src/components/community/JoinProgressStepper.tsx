import { Component, For, Show } from "solid-js";
import { joinProgress, type JoinStageStatus } from "../../stores/join.store";

/**
 * Display-only "dial-in" stepper for the community join flow. Reads the
 * `joinProgress` store, which mirrors the backend's per-phase
 * `joinProgress` events. Each phase shows a status dot (modeled on
 * NetworkIndicator) plus its backend-supplied label. All timeout and
 * gating logic lives in the Rust `join_gate` module; this component
 * never decides anything — it only renders the stream.
 */
function dotClass(status: JoinStageStatus): string {
  return `join-step-dot join-step-dot-${status}`;
}

const JoinProgressStepper: Component = () => (
  <Show when={joinProgress.active || joinProgress.stages.length > 0}>
    <ul class="join-stepper">
      <For each={joinProgress.stages}>
        {(stage) => (
          <li class="join-stepper-row">
            <span class={dotClass(stage.status)} />
            <span class="join-step-label">{stage.label}</span>
          </li>
        )}
      </For>
      <Show when={joinProgress.active && joinProgress.stages.length === 0}>
        <li class="join-stepper-row">
          <span class="join-step-dot join-step-dot-started" />
          <span class="join-step-label">Connecting…</span>
        </li>
      </Show>
    </ul>
  </Show>
);

export default JoinProgressStepper;
