import { Component, For, Show, createSignal, createEffect, onCleanup } from "solid-js";
import {
  handleCreateCommunityInvite,
  handleRevokeCommunityInvite,
  handleListCommunityInvites,
} from "../../../handlers/community.handlers";
import { communityState } from "../../../stores/community.store";
import type { InviteDto } from "../../../stores/types";
import { formatExpiry } from "../../../utils/formatting";
import { addToast } from "../../../stores/toast.store";

interface InvitesTabProps {
  communityId: string;
  canCreateInvite: boolean;
  canManage: boolean;
}

const InvitesTab: Component<InvitesTabProps> = (props) => {
  const invites = (): InviteDto[] => communityState.communityInvites[props.communityId] ?? [];
  const [loaded, setLoaded] = createSignal(false);
  const [creatingInvite, setCreatingInvite] = createSignal(false);
  const [maxUses, setMaxUses] = createSignal("");
  const [expiresIn, setExpiresIn] = createSignal("");
  const [copiedHash, setCopiedHash] = createSignal<string | null>(null);
  const [revokingHash, setRevokingHash] = createSignal<string | null>(null);

  // Force expiry display refresh every 60s
  const [, setTick] = createSignal(0);
  const expiryTimer = setInterval(() => setTick((t) => t + 1), 60_000);
  onCleanup(() => clearInterval(expiryTimer));

  createEffect(() => {
    if ((props.canManage || props.canCreateInvite) && !loaded()) {
      setLoaded(true);
      handleListCommunityInvites(props.communityId);
    }
  });

  async function submitInvite(): Promise<void> {
    const parsedMaxUses = maxUses() !== "" ? parseInt(maxUses(), 10) : undefined;
    const parsedExpiresIn = expiresIn() !== "" ? parseInt(expiresIn(), 10) : undefined;
    const result = await handleCreateCommunityInvite(props.communityId, parsedMaxUses, parsedExpiresIn);
    if (result) {
      // Build invite link: rekindle://invite/{manifestKey}/{code}
      const link = `rekindle://invite/${result.manifestKey}/${result.code}`;
      try { await navigator.clipboard.writeText(link); } catch {}
      addToast("Invite link copied to clipboard", "success");
      setCreatingInvite(false);
      setMaxUses("");
      setExpiresIn("");
    }
    // Error toast handled by handler
  }

  async function revokeInvite(codeHash: string): Promise<void> {
    setRevokingHash(codeHash);
    try {
      const ok = await handleRevokeCommunityInvite(props.communityId, codeHash);
      if (ok) addToast("Invite revoked", "success");
      // Error toast handled by handler; store update handled by handler
    } finally {
      setRevokingHash(null);
    }
  }

  function truncateHash(hash: string): string {
    if (hash.length > 12) return hash.slice(0, 12) + "...";
    return hash;
  }

  return (
    <div class="settings-section">
      <Show when={props.canCreateInvite}>
        <Show
          when={creatingInvite()}
          fallback={
            <button class="form-btn-primary" onClick={() => setCreatingInvite(true)}>
              Create Invite
            </button>
          }
        >
          <div class="invite-create-form">
            <input
              class="form-input"
              type="number"
              min="1"
              placeholder="Unlimited"
              value={maxUses()}
              onInput={(e) => setMaxUses(e.currentTarget.value)}
            />
            <select
              class="form-select"
              value={expiresIn()}
              onChange={(e) => setExpiresIn(e.currentTarget.value)}
            >
              <option value="">Never</option>
              <option value="1800">30 minutes</option>
              <option value="3600">1 hour</option>
              <option value="21600">6 hours</option>
              <option value="43200">12 hours</option>
              <option value="86400">1 day</option>
              <option value="604800">7 days</option>
            </select>
            <button class="form-btn-primary" onClick={submitInvite}>
              Create
            </button>
            <button class="form-btn-secondary" onClick={() => setCreatingInvite(false)}>
              Cancel
            </button>
          </div>
        </Show>
      </Show>

      <Show when={invites().length > 0} fallback={
        <div class="settings-hint">No invites yet.</div>
      }>
        <For each={invites()}>
          {(invite) => (
            <div class="settings-list-item">
              <div class="settings-list-info">
                <span class="settings-list-name">{truncateHash(invite.codeHash)}</span>
                <span class="settings-list-role">
                  by {(() => {
                    const community = communityState.communities[props.communityId];
                    const member = community?.members.find((m) => m.pseudonymKey === invite.createdBy);
                    return member?.displayName ?? (invite.createdBy.slice(0, 8) + "...");
                  })()}
                </span>
                <span class="settings-list-date">
                  Uses: {invite.uses} / {invite.maxUses !== null ? invite.maxUses : "\u221E"}
                  {" \u00B7 "}
                  Expires: {formatExpiry(invite.expiresAt)}
                  <Show when={invite.expiresAt !== null && invite.expiresAt <= Math.floor(Date.now() / 1000)}>
                    {" (expired)"}
                  </Show>
                </span>
              </div>
              <Show when={props.canManage}>
                <button
                  class="form-btn-secondary"
                  onClick={() => revokeInvite(invite.codeHash)}
                  disabled={revokingHash() === invite.codeHash}
                >
                  {revokingHash() === invite.codeHash ? "Revoking..." : "Revoke"}
                </button>
              </Show>
            </div>
          )}
        </For>
      </Show>
    </div>
  );
};

export default InvitesTab;
