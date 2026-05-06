import { Component, Show, createEffect, createMemo, createSignal } from "solid-js";
import type { Community } from "../../../stores/community.store";
import type { ConfirmOptions } from "../CommunitySettingsModal";
import { handleRotateMek } from "../../../handlers/community.handlers";
import { addToast } from "../../../stores/toast.store";
import { commands, type CommunityPolicy } from "../../../ipc/commands";
import {
  calculateBasePermissions,
  hasPermission,
  MANAGE_COMMUNITY,
} from "../../../ipc/permissions";
import { ICON_KEY } from "../../../icons";
import FormField from "../../common/FormField";

interface SecurityTabProps {
  community: Community;
  requestConfirm: (opts: ConfirmOptions) => void;
}

const SecurityTab: Component<SecurityTabProps> = (props) => {
  const [rotating, setRotating] = createSignal(false);
  const [expanding, setExpanding] = createSignal(false);

  // Architecture §10.7 + §20.6 — community-wide policy editor
  // (raid-protection thresholds + optional rules text). Loaded on mount,
  // saved on explicit click.
  const [policyText, setPolicyText] = createSignal("");
  const [maxJoins, setMaxJoins] = createSignal(20);
  const [intervalSecs, setIntervalSecs] = createSignal(600);
  const [policyLoaded, setPolicyLoaded] = createSignal(false);
  const [savingPolicy, setSavingPolicy] = createSignal(false);

  // Architecture §17.1 tier 2 — community default notification level.
  const [defaultNotifLevel, setDefaultNotifLevel] = createSignal<"all" | "mentions" | "nothing">("all");
  const [savingDefaultLevel, setSavingDefaultLevel] = createSignal(false);

  const canManageCommunity = createMemo(() => {
    const perms = calculateBasePermissions(props.community.myRoleIds, props.community.roles);
    return hasPermission(perms, MANAGE_COMMUNITY);
  });

  // Architecture §15 — Plate Gate scaling: when segment 0 (or the
  // highest segment) is full, an admin can write a SegmentAdded
  // governance entry that allocates the next segment. We approximate
  // "full" by checking if local member count >= 255 (the SMPL slot
  // limit per Veilid; documented at architecture line 126).
  const isHighestSegmentFull = createMemo(() => (props.community.members?.length ?? 0) >= 255);

  createEffect(() => {
    if (props.community.id) {
      void commands.getCommunityPolicy(props.community.id).then((policy: CommunityPolicy) => {
        setPolicyText(policy.policyText ?? "");
        setMaxJoins(policy.maxJoinsPerInterval);
        setIntervalSecs(policy.joinIntervalSeconds);
        setPolicyLoaded(true);
      }).catch((e) => {
        console.error("Failed to load community policy:", e);
        setPolicyLoaded(true);
      });
      void commands.getCommunityDefaultNotificationLevel(props.community.id)
        .then((level) => {
          if (level === "all" || level === "mentions" || level === "nothing") {
            setDefaultNotifLevel(level);
          }
        })
        .catch((e) => console.error("Failed to load community default notification level:", e));
    }
  });

  function confirmRotateKey(): void {
    props.requestConfirm({
      title: "Rotate Encryption Key",
      message: "Generate a new Media Encryption Key? All members will automatically receive the new key.",
      confirmLabel: "Rotate",
      action: async () => {
        setRotating(true);
        try {
          await handleRotateMek(props.community.id);
          addToast("Encryption key rotated", "success");
        } finally {
          setRotating(false);
        }
      },
    });
  }

  async function expandSegment(): Promise<void> {
    setExpanding(true);
    try {
      const newSegmentIndex = await commands.expandCommunitySegment(props.community.id);
      addToast(`Segment ${newSegmentIndex} created — new members will join here`, "success");
    } catch (e) {
      console.error("Failed to expand segment:", e);
      addToast(typeof e === "string" ? e : "Failed to expand segment", "error");
    } finally {
      setExpanding(false);
    }
  }

  async function savePolicy(): Promise<void> {
    if (maxJoins() < 1 || intervalSecs() < 1) {
      addToast("Threshold values must be ≥1", "error");
      return;
    }
    setSavingPolicy(true);
    try {
      const text = policyText().trim();
      await commands.setCommunityPolicy(
        props.community.id,
        text.length > 0 ? text : null,
        maxJoins(),
        intervalSecs(),
      );
      addToast("Community policy saved", "success");
    } catch (e) {
      console.error("Failed to save community policy:", e);
      addToast(typeof e === "string" ? e : "Failed to save policy", "error");
    } finally {
      setSavingPolicy(false);
    }
  }

  async function saveDefaultLevel(level: "all" | "mentions" | "nothing"): Promise<void> {
    setDefaultNotifLevel(level);
    setSavingDefaultLevel(true);
    try {
      await commands.setCommunityDefaultNotificationLevel(props.community.id, level);
      addToast(`Default notifications: ${level}`, "success");
    } catch (e) {
      console.error("Failed to set community default notification level:", e);
      addToast("Failed to update default notifications", "error");
    } finally {
      setSavingDefaultLevel(false);
    }
  }

  return (
    <div class="settings-section">
      <FormField label="MEK Generation">
        <div class="settings-value">
          {props.community.mekGeneration}
          <span class="settings-hint-inline"> (higher = more recent)</span>
        </div>
      </FormField>
      <FormField label="Encryption Key Rotation">
        <div class="settings-hint">
          Rotating the encryption key generates a new Media Encryption Key (MEK).
          All members will automatically receive the new key. Messages encrypted
          with previous keys remain readable.
        </div>
        <button class="form-btn-danger" onClick={confirmRotateKey} disabled={rotating()}>
          <span class="nf-icon">{ICON_KEY}</span> {rotating() ? "Rotating..." : "Rotate Encryption Key"}
        </button>
      </FormField>
      <FormField label="Network Model">
        <div class="settings-value">
          Flat SMPL Governance (P2P)
        </div>
      </FormField>

      {/* Architecture §17.1 tier 2 — community-wide default notification level. */}
      <Show when={canManageCommunity()}>
        <FormField label="Default Notifications (community-wide)">
          <div class="settings-hint">
            Sets the default notification level for every channel in this
            community. Members can still override per-channel locally
            (architecture §17.1 tier 1).
          </div>
          <select
            class="form-select"
            value={defaultNotifLevel()}
            disabled={savingDefaultLevel()}
            onChange={(e) => void saveDefaultLevel(e.currentTarget.value as "all" | "mentions" | "nothing")}
          >
            <option value="all">All messages</option>
            <option value="mentions">Mentions only</option>
            <option value="nothing">Nothing</option>
          </select>
        </FormField>
      </Show>

      {/* Architecture §10.7 + §20.6 — policy editor. */}
      <Show when={canManageCommunity()}>
        <FormField label="Raid-protection thresholds (architecture §20.6)">
          <div class="settings-hint">
            Each peer alerts moderators when joins exceed
            <strong> {maxJoins()} </strong> in the last
            <strong> {intervalSecs()} </strong> seconds. Defaults are 20
            joins per 600 s.
          </div>
          <div class="form-field-row">
            <input
              class="form-input"
              type="number"
              min={1}
              value={maxJoins()}
              disabled={!policyLoaded() || savingPolicy()}
              onInput={(e) => setMaxJoins(parseInt(e.currentTarget.value, 10) || 0)}
            />
            <input
              class="form-input"
              type="number"
              min={1}
              value={intervalSecs()}
              disabled={!policyLoaded() || savingPolicy()}
              onInput={(e) => setIntervalSecs(parseInt(e.currentTarget.value, 10) || 0)}
            />
          </div>
        </FormField>
        <FormField label="Community rules (optional Markdown)">
          <textarea
            class="form-input"
            rows={6}
            value={policyText()}
            disabled={!policyLoaded() || savingPolicy()}
            onInput={(e) => setPolicyText(e.currentTarget.value)}
          />
          <button class="form-btn-primary" onClick={() => void savePolicy()} disabled={savingPolicy()}>
            {savingPolicy() ? "Saving..." : "Save policy"}
          </button>
        </FormField>
      </Show>

      {/* Architecture §15 — Plate Gate admin expansion. */}
      <Show when={canManageCommunity()}>
        <FormField label="Membership Capacity">
          <div class="settings-hint">
            Each segment holds 255 member slots. When the highest
            segment fills, expanding writes a `SegmentAdded` governance
            entry that allocates a new SMPL record so new joiners have a
            slot to claim.
          </div>
          <button
            class="form-btn-secondary"
            onClick={() => void expandSegment()}
            disabled={!isHighestSegmentFull() || expanding()}
            title={
              isHighestSegmentFull()
                ? "Allocate the next 255-slot segment"
                : "Available once the highest segment is full"
            }
          >
            {expanding() ? "Expanding..." : "Expand to new segment"}
          </button>
        </FormField>
      </Show>
    </div>
  );
};

export default SecurityTab;
