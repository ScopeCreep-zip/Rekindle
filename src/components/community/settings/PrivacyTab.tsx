import { Component, createEffect, createSignal } from "solid-js";
import type { Community } from "../../../stores/community.store";
import { commands, type PresenceSharingPolicy } from "../../../ipc/commands";
import { addToast } from "../../../stores/toast.store";
import FormField from "../../common/FormField";

interface PrivacyTabProps {
  community: Community;
}

const DEFAULT_POLICY: PresenceSharingPolicy = {
  shareOnline: true,
  shareLocation: "none",
  shareActivity: "none",
  lastSeen: "hidden",
};

/**
 * Per-member presence consent (default-deny). Every shareable signal is
 * off until the user opts in — and by reciprocity, a signal you don't
 * share you don't get to read off peers. Changes republish immediately so
 * opting out redacts the next presence write right away.
 */
const PrivacyTab: Component<PrivacyTabProps> = (props) => {
  const [policy, setPolicy] = createSignal<PresenceSharingPolicy>(DEFAULT_POLICY);
  const [loaded, setLoaded] = createSignal(false);
  const [saving, setSaving] = createSignal(false);

  createEffect(() => {
    const id = props.community.id;
    if (!id) return;
    void commands.getPresencePolicy(id)
      .then((p) => {
        setPolicy(p);
        setLoaded(true);
      })
      .catch((e) => {
        console.error("Failed to load presence policy:", e);
        setLoaded(true);
      });
  });

  async function persist(next: PresenceSharingPolicy): Promise<void> {
    setPolicy(next);
    setSaving(true);
    try {
      await commands.setPresencePolicy(props.community.id, next);
    } catch (e) {
      console.error("Failed to save presence policy:", e);
      addToast(typeof e === "string" ? e : "Failed to save privacy settings", "error");
    } finally {
      setSaving(false);
    }
  }

  const disabled = (): boolean => !loaded() || saving();

  return (
    <div class="settings-section">
      <FormField label="Appear online">
        <div class="settings-hint">
          When off, you appear offline to everyone here while staying fully
          connected (Invisible). This is the only signal shared by default —
          everything below is off until you turn it on.
        </div>
        <label class="automod-enabled-toggle">
          <input
            type="checkbox"
            checked={policy().shareOnline}
            disabled={disabled()}
            onChange={(e) => void persist({ ...policy(), shareOnline: e.currentTarget.checked })}
          />
          <span>{policy().shareOnline ? "Online to members" : "Invisible"}</span>
        </label>
      </FormField>

      <FormField label="Share which channel you're in">
        <div class="settings-hint">
          Others can't see what channel you're in unless you turn this on. If
          you don't share it, you won't see where other members are either.
        </div>
        <select
          class="form-select"
          value={policy().shareLocation}
          disabled={disabled()}
          onChange={(e) =>
            void persist({ ...policy(), shareLocation: e.currentTarget.value as PresenceSharingPolicy["shareLocation"] })
          }
        >
          <option value="none">Don't share</option>
          <option value="members">Share with members</option>
        </select>
      </FormField>

      <FormField label="Share activity / game">
        <div class="settings-hint">
          Share what you're playing or doing. Off by default; reciprocal —
          hidden both ways unless shared.
        </div>
        <select
          class="form-select"
          value={policy().shareActivity}
          disabled={disabled()}
          onChange={(e) =>
            void persist({ ...policy(), shareActivity: e.currentTarget.value as PresenceSharingPolicy["shareActivity"] })
          }
        >
          <option value="none">Don't share</option>
          <option value="members">Share with members</option>
        </select>
      </FormField>

      <FormField label="Last seen">
        <div class="settings-hint">
          Hidden by default. "Recently" shows only an hour-coarse bucket;
          "Exact time" reveals a precise timestamp. You see peers' last-seen
          only at the precision you expose yourself.
        </div>
        <select
          class="form-select"
          value={policy().lastSeen}
          disabled={disabled()}
          onChange={(e) =>
            void persist({ ...policy(), lastSeen: e.currentTarget.value as PresenceSharingPolicy["lastSeen"] })
          }
        >
          <option value="hidden">Hidden</option>
          <option value="coarse">Recently / a while ago</option>
          <option value="exact">Exact time</option>
        </select>
      </FormField>
    </div>
  );
};

export default PrivacyTab;
