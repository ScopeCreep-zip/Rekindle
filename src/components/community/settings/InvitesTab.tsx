import { type Component, For, Show, createSignal, createEffect, onCleanup } from "solid-js";
import {
  handleCreateCommunityInvite,
  handleRevokeCommunityInvite,
  handleListCommunityInvites,
} from "../../../actions/community.actions";
import { communityState } from "../../../stores/community.store";
import type { InviteDto } from "../../../ipc/commands/dto";
import { formatExpiry } from "../../../utils/formatting";
import { addToast } from "../../../stores/toast.store";

interface InvitesTabProps {
  communityId: string;
  canCreateInvite: boolean;
  canManage: boolean;
}

/** The redemptions an invite may allow (IJ1): explicit choices, never a
 *  blank field. Every reader refuses an invite outside 1–100
 *  (`invite_quota::check_max_uses_cap`: no unlimited invites), so 100 is
 *  the most one allows. */
const MAX_USES_CHOICES = [1, 5, 10, 25, 50, 100] as const;
/** Single use, shown plainly (fail-closed for vulnerable communities). */
const DEFAULT_MAX_USES = 1;

const InvitesTab: Component<InvitesTabProps> = (props) => {
  const invites = (): InviteDto[] => communityState.communityInvites[props.communityId] ?? [];
  const [loaded, setLoaded] = createSignal(false);
  const [creatingInvite, setCreatingInvite] = createSignal(false);
  const [maxUses, setMaxUses] = createSignal(DEFAULT_MAX_USES);
  const [expiresIn, setExpiresIn] = createSignal("604800");
  const [copiedHash, setCopiedHash] = createSignal<string | null>(null);
  const [revokingHash, setRevokingHash] = createSignal<string | null>(null);
  const [createdLink, setCreatedLink] = createSignal<string | null>(null);

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

  async function copyToClipboard(text: string, hash?: string, what = "Invite link"): Promise<void> {
    try {
      await navigator.clipboard.writeText(text);
      if (hash) {
        setCopiedHash(hash);
        setTimeout(() => setCopiedHash(null), 2000);
      }
      addToast(`${what} copied!`, "success");
    } catch {
      addToast("Failed to copy — select and copy manually", "error");
    }
  }

  async function submitInvite(): Promise<void> {
    const parsedExpiresIn = expiresIn() !== "" ? parseInt(expiresIn(), 10) : undefined;
    const result = await handleCreateCommunityInvite(props.communityId, maxUses(), parsedExpiresIn);
    if (result) {
      setCreatedLink(result.url);
      await copyToClipboard(result.url);
      setCreatingInvite(false);
      setMaxUses(DEFAULT_MAX_USES);
      setExpiresIn("604800");
    }
  }

  async function revokeInvite(inviteId: string): Promise<void> {
    setRevokingHash(inviteId);
    try {
      const ok = await handleRevokeCommunityInvite(props.communityId, inviteId);
      if (ok) addToast("Invite revoked", "success");
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
            <label class="invite-max-uses">
              <span class="form-field-label">Max uses</span>
              <select
                class="form-select"
                value={String(maxUses())}
                onChange={(e) => setMaxUses(parseInt(e.currentTarget.value, 10))}
              >
                <For each={MAX_USES_CHOICES}>
                  {(n) => <option value={String(n)}>{n === 1 ? "1 use" : `${n} uses`}</option>}
                </For>
              </select>
            </label>
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

      {/* Show newly created invite link prominently */}
      <Show when={createdLink()}>
        <div class="invite-link-display">
          <input
            class="form-input"
            type="text"
            readOnly
            value={createdLink()!}
            onClick={(e) => e.currentTarget.select()}
          />
          <button
            class="form-btn-primary"
            onClick={() => copyToClipboard(createdLink()!)}
          >
            Copy Link
          </button>
          <button
            class="form-btn-secondary"
            onClick={() => setCreatedLink(null)}
          >
            Dismiss
          </button>
        </div>
      </Show>

      <Show when={invites().length > 0} fallback={
        <div class="settings-hint">No invites yet.</div>
      }>
        <div class="settings-hint">
          Ask the person joining to check this matches what their screen shows.
        </div>
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
                <span class="invite-fingerprint">
                  Fingerprint: <code>{invite.fingerprint}</code>
                </span>
                <span class="settings-list-date">
                  Uses: {invite.uses} / {invite.maxUses}
                  {" \u00B7 "}
                  Expires: {formatExpiry(invite.expiresAt)}
                  <Show when={invite.expiresAt !== null && invite.expiresAt <= Math.floor(Date.now() / 1000)}>
                    {" (expired)"}
                  </Show>
                </span>
              </div>
              <div class="settings-list-actions">
                <button
                  class="form-btn-secondary"
                  onClick={() => copyToClipboard(invite.fingerprint, undefined, "Fingerprint")}
                >
                  Copy Fingerprint
                </button>
                <Show when={invite.url}>
                  {(url) => (
                  <button
                    class="form-btn-secondary"
                    onClick={() => copyToClipboard(url(), invite.codeHash)}
                  >
                    {copiedHash() === invite.codeHash ? "Copied!" : "Copy Link"}
                  </button>
                  )}
                </Show>
                <Show when={props.canManage}>
                  <button
                    class="form-btn-secondary"
                    onClick={() => revokeInvite(invite.inviteId)}
                    disabled={revokingHash() === invite.inviteId}
                  >
                    {revokingHash() === invite.inviteId ? "Revoking..." : "Revoke"}
                  </button>
                </Show>
              </div>
            </div>
          )}
        </For>
      </Show>
    </div>
  );
};

export default InvitesTab;
