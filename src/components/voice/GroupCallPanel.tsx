import { Component, For, Show, createEffect, createSignal, onCleanup } from "solid-js";
import { callsState } from "../../stores/calls.store";
import { friendsState } from "../../stores/friends.store";
import {
  handleAcceptGroupCall,
  handleDeclineGroupCall,
  handleEndGroupCall,
} from "../../handlers/calls.handlers";
import { ICON_HANGUP } from "../../icons";

// Wave 12 W12.10 — group call surfaces.
//
// Two distinct panels share this file:
//   - <GroupCallIncomingBanner /> renders a top banner when an
//     incoming group offer is in `incomingGroupCalls[0]`. Accept /
//     Decline routes through the W12.9 backend commands.
//   - <ActiveGroupCallPanel /> mounts when the local user has accepted
//     (or initiated) a group call: shows a tile per participant with
//     accepted/ringing status + a hangup button.
//
// The grid intentionally stays simple in this drop. Per-participant
// volume sliders + pin/spotlight are good follow-ups; the protocol
// already carries everything they need.

const GroupCallIncomingBanner: Component = () => {
  const head = () => callsState.incomingGroupCalls[0];

  const [now, setNow] = createSignal(Date.now());
  let interval: ReturnType<typeof setInterval> | undefined;
  createEffect(() => {
    if (head()) {
      interval = setInterval(() => setNow(Date.now()), 1000);
      onCleanup(() => {
        if (interval) clearInterval(interval);
        interval = undefined;
      });
    }
  });
  const remaining = (): number => {
    const e = head();
    if (!e) return 0;
    return Math.max(0, Math.ceil((e.expiresAtMs - now()) / 1000));
  };

  return (
    <Show when={head()}>
      {(entry) => (
        <div class="group-call-incoming-banner" role="alertdialog" aria-live="polite">
          <div class="group-call-banner-text">
            <div class="group-call-banner-title">
              {entry().displayName} is starting a group {entry().kind} call
            </div>
            <div class="group-call-banner-meta">
              {entry().participants.length} participant
              {entry().participants.length === 1 ? "" : "s"} · {remaining()}s
            </div>
          </div>
          <div class="group-call-banner-actions">
            <button
              type="button"
              class="form-btn-primary"
              onClick={() => void handleAcceptGroupCall(entry().callId)}
            >
              Accept
            </button>
            <button
              type="button"
              class="form-btn-secondary"
              onClick={() => void handleDeclineGroupCall(entry().callId)}
            >
              Decline
            </button>
          </div>
        </div>
      )}
    </Show>
  );
};

const ActiveGroupCallPanel: Component = () => {
  const call = () => callsState.activeGroupCall;

  const [now, setNow] = createSignal(Date.now());
  let interval: ReturnType<typeof setInterval> | undefined;
  createEffect(() => {
    if (call()) {
      interval = setInterval(() => setNow(Date.now()), 1000);
      onCleanup(() => {
        if (interval) clearInterval(interval);
        interval = undefined;
      });
    }
  });

  const elapsed = (): string => {
    const c = call();
    if (!c) return "0:00";
    const total = Math.max(0, Math.floor((now() - c.startedAtMs) / 1000));
    const m = Math.floor(total / 60);
    const s = total % 60;
    return `${m}:${s.toString().padStart(2, "0")}`;
  };

  const participantName = (key: string): string => {
    return friendsState.friends[key]?.displayName ?? key.slice(0, 8) + "…";
  };

  return (
    <Show when={call()}>
      {(c) => (
        <div class="group-call-panel" role="region" aria-label="Group call in progress">
          <div class="group-call-panel-header">
            <span class="group-call-panel-title">
              Group {c().kind} call · {c().accepted.length}/{c().participants.length}
            </span>
            <span class="active-call-timer">{elapsed()}</span>
          </div>
          <div class="group-call-grid">
            <For each={c().participants}>
              {(pubkey) => {
                const accepted = () => c().accepted.includes(pubkey);
                return (
                  <div
                    class="group-call-tile"
                    classList={{
                      "group-call-tile-active": accepted(),
                      "group-call-tile-ringing": !accepted(),
                    }}
                  >
                    <div class="group-call-tile-avatar">
                      {participantName(pubkey).slice(0, 1).toUpperCase()}
                    </div>
                    <div class="group-call-tile-name">{participantName(pubkey)}</div>
                    <div class="group-call-tile-status">
                      {accepted() ? "in call" : "ringing…"}
                    </div>
                  </div>
                );
              }}
            </For>
          </div>
          <div class="group-call-controls">
            <button
              type="button"
              class="call-control-btn call-control-btn-hangup"
              onClick={() => void handleEndGroupCall(c().callId)}
              aria-label="Leave call"
              title="Leave call"
            >
              <span class="nf-icon" aria-hidden="true">{ICON_HANGUP}</span>
            </button>
          </div>
        </div>
      )}
    </Show>
  );
};

const GroupCallPanel: Component = () => {
  return (
    <>
      <GroupCallIncomingBanner />
      <ActiveGroupCallPanel />
    </>
  );
};

export default GroupCallPanel;
