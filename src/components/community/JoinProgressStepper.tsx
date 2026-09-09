import { Component, For, Show, onMount, onCleanup } from "solid-js";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { subscribeCommunityEvents } from "../../ipc/channels";
import { isLegacyCommunityEvent } from "../../ipc/channels/community_events";
import { joinProgress, applyJoinProgress, type JoinStageStatus } from "../../stores/join.store";

/**
 * Display-only "dial-in" stepper for the community join flow. Mirrors
 * the backend's per-phase `joinProgress` events; each phase shows a
 * status dot (modeled on NetworkIndicator) plus its backend-supplied
 * label. All timeout/gating logic lives in the Rust `join_gate` module.
 *
 * The join modal is launched from the BuddyListWindow, which does NOT
 * run the full community-event dispatcher (that lives in the
 * CommunityWindow). `joinProgress` is broadcast to every window, so we
 * subscribe directly here while mounted and feed the local store — that
 * way the stepper updates regardless of which window hosts the modal.
 */
function dotClass(status: JoinStageStatus): string {
  return `join-step-dot join-step-dot-${status}`;
}

const JoinProgressStepper: Component = () => {
  let unlisten: UnlistenFn | undefined;

  onMount(() => {
    void subscribeCommunityEvents((event) => {
      // Join progress moved to the daemon vocabulary; the legacy
      // envelope no longer carries it.
      if (
        !isLegacyCommunityEvent(event) &&
        "membership" in event &&
        "joinProgress" in event.membership
      ) {
        const { stage, status } = event.membership.joinProgress;
        applyJoinProgress(stage, status as JoinStageStatus);
      }
    }).then((fn) => {
      unlisten = fn;
    });
  });

  onCleanup(() => unlisten?.());

  return (
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
};

export default JoinProgressStepper;
