import { type Component, createResource, createSignal, createUniqueId, Show } from "solid-js";
import {
  commands,
  type PresencePolicyField,
  type PresenceSharingPolicy,
} from "../../../ipc/commands";
import type { Community } from "../../../stores/community.store";
import { addToast } from "../../../stores/toast.store";
import FormField from "../../common/FormField";

interface PrivacyTabProps {
  community: Community;
}

/**
 * Per-member presence consent (default-deny). Every shareable signal is
 * off until the user opts in — and by reciprocity, a signal you don't
 * share you don't get to read off peers. Changes republish immediately so
 * opting out redacts the next presence write right away.
 */
const PrivacyTab: Component<PrivacyTabProps> = (props) => {
  const channelPresenceId = createUniqueId();
  const activityPresenceId = createUniqueId();
  const lastSeenId = createUniqueId();
  const [policy, { refetch }] = createResource(
    () => props.community.id,
    (id) => commands.getPresencePolicy(id)
  );
  const [saving, setSaving] = createSignal(false);

  async function persist(field: PresencePolicyField): Promise<void> {
    setSaving(true);
    try {
      await commands.setPresencePolicyField(props.community.id, field);
    } catch (e) {
      console.error("Failed to save presence policy:", e);
      addToast(typeof e === "string" ? e : "Failed to save privacy settings", "error");
    } finally {
      try {
        await refetch();
      } finally {
        setSaving(false);
      }
    }
  }

  const policyLoading = () => policy.loading;

  return (
    <div class="settings-section">
      <Show when={policy.error} fallback={<Show when={policy.latest}>{renderPolicy}</Show>}>
        <div role="alert">Privacy settings could not be loaded.</div>
        <button type="button" class="btn-secondary" onClick={() => void refetch()}>
          Retry
        </button>
      </Show>
    </div>
  );

  function renderPolicy(policy: () => PresenceSharingPolicy) {
    return (
      <>
        <FormField label="Appear online">
          <div class="settings-hint">
            When off, you appear offline to everyone here while staying fully connected (Invisible).
            This is the only signal shared by default — everything below is off until you turn it
            on.
          </div>
          <label class="automod-enabled-toggle">
            <input
              type="checkbox"
              checked={policy().shareOnline}
              disabled={saving() || policyLoading()}
              onChange={(e) => void persist({ shareOnline: e.currentTarget.checked })}
            />
            <span>{policy().shareOnline ? "Online to members" : "Invisible"}</span>
          </label>
        </FormField>

        <FormField label="Share which channel you're in" controlId={channelPresenceId}>
          <div class="settings-hint">
            Channel presence badges show where members are focused when both sides share. Joining a
            voice call separately shares your call membership with the community.
          </div>
          <select
            id={channelPresenceId}
            class="form-select"
            value={policy().shareLocation}
            disabled={saving() || policyLoading()}
            onChange={(e) =>
              void persist({
                shareLocation: e.currentTarget.value as PresenceSharingPolicy["shareLocation"],
              })
            }
          >
            <option value="none">Don't share</option>
            <option value="members">Share with members</option>
          </select>
        </FormField>

        <FormField label="Share activity / game" controlId={activityPresenceId}>
          <div class="settings-hint">
            Share what you're playing or doing. Off by default; reciprocal — hidden both ways unless
            shared.
          </div>
          <select
            id={activityPresenceId}
            class="form-select"
            value={policy().shareActivity}
            disabled={saving() || policyLoading()}
            onChange={(e) =>
              void persist({
                shareActivity: e.currentTarget.value as PresenceSharingPolicy["shareActivity"],
              })
            }
          >
            <option value="none">Don't share</option>
            <option value="members">Share with members</option>
          </select>
        </FormField>

        <FormField label="Last seen" controlId={lastSeenId}>
          <div class="settings-hint">
            Hidden by default. "Recently" shows only an hour-coarse bucket; "Exact time" reveals a
            precise timestamp. You see peers' last-seen only at the precision you expose yourself.
          </div>
          <select
            id={lastSeenId}
            class="form-select"
            value={policy().lastSeen}
            disabled={saving() || policyLoading()}
            onChange={(e) =>
              void persist({
                lastSeen: e.currentTarget.value as PresenceSharingPolicy["lastSeen"],
              })
            }
          >
            <option value="hidden">Hidden</option>
            <option value="coarse">Recently / a while ago</option>
            <option value="exact">Exact time</option>
          </select>
        </FormField>
      </>
    );
  }
};

export default PrivacyTab;
