import { type Component, Show, createSignal } from "solid-js";
import { commands } from "../../ipc/commands";
import { isCommandError } from "../../ipc/commands/dto";
import { communityState, setCommunityState } from "../../stores/community.store";
import { errorMessage } from "../../utils/error";

/**
 * This device's standing in the community (plan E3.2, ADR 0022 rule 4,
 * lead decision 63): "Re-enrol this device" is always listed in Community
 * Settings, enabled only while the Host says the device `needsReenrol` (a
 * fresh device, or one whose person re-joined from another device); the
 * main pane shows the same action as a notice only then. It enrols through
 * the Host's review (`CommunityDeviceEnrol`, TrustChanging), then reads the
 * Host's standing again.
 *
 * `notice`: the main pane's notice; otherwise the Settings row.
 */
const CommunityDeviceStatus: Component<{ communityId: string; notice?: boolean }> = (props) => {
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  // Both places can be open in one window: each names its own elements.
  const reasonId = (): string => `community-device-reason-${props.notice ? "notice" : "settings"}`;
  const needs = (): boolean =>
    communityState.communities[props.communityId]?.deviceStatus === "needsReenrol";

  async function enrol(): Promise<void> {
    if (!needs() || busy()) return;
    setBusy(true);
    setError(null);
    try {
      await commands.communityDeviceEnrol(props.communityId);
      const detail = (await commands.getCommunityDetails()).find((d) => d.id === props.communityId);
      if (detail)
        setCommunityState("communities", props.communityId, "deviceStatus", detail.deviceStatus);
    } catch (e) {
      if (!(isCommandError(e) && e.type === "cancelled")) setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Show when={!props.notice || needs()}>
      <div class="community-device-status">
        <span
          class={needs() ? "community-device-notice" : "sr-only"}
          role={needs() ? "status" : undefined}
          id={reasonId()}
        >
          {needs()
            ? "This device can't post here until it is enrolled again."
            : "This device is enrolled."}
        </span>
        <button
          type="button"
          class="form-btn-secondary community-device-enrol"
          disabled={!needs() || busy()}
          aria-describedby={reasonId()}
          onClick={() => void enrol()}
        >
          {busy() ? "Enrolling…" : "Re-enrol this device"}
        </button>
        <Show when={error()}>
          {(message) => (
            <span class="form-error" role="alert">
              {message()}
            </span>
          )}
        </Show>
      </div>
    </Show>
  );
};

export default CommunityDeviceStatus;
