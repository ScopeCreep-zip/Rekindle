import { Component, Show, For, createSignal } from "solid-js";
import { friendsState } from "../../stores/friends.store";
import {
  handleGenerateInvite,
  handleAddFriendFromInvite,
  handleCancelInvite,
} from "../../handlers/buddy.handlers";
import { maskInviteUrl } from "../../utils/masking";
import { formatRelativeTime, formatTimeUntil } from "../../utils/time";

interface InviteLinkTabProps {
  onClose: () => void;
}

const InviteLinkTab: Component<InviteLinkTabProps> = (props) => {
  const [inviteString, setInviteString] = createSignal("");
  const [generatedInvite, setGeneratedInvite] = createSignal("");
  const [currentInviteId, setCurrentInviteId] = createSignal<string | null>(null);
  const [generating, setGenerating] = createSignal(false);
  const [copied, setCopied] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  async function handleGenerate(): Promise<void> {
    setGenerating(true);
    setError(null);
    const result = await handleGenerateInvite();
    setGenerating(false);
    if (typeof result === "string") {
      setError(result);
    } else {
      setGeneratedInvite(result.url);
      setCurrentInviteId(result.inviteId);
    }
  }

  async function handleCancelCurrentInvite(): Promise<void> {
    const inviteId = currentInviteId();
    if (!inviteId) return;
    setError(null);
    const err = await handleCancelInvite(inviteId);
    if (err) {
      setError(err);
    } else {
      setGeneratedInvite("");
      setCurrentInviteId(null);
    }
  }

  async function handleCancelListInvite(inviteId: string): Promise<void> {
    setError(null);
    const err = await handleCancelInvite(inviteId);
    if (err) {
      setError(err);
    }
    if (currentInviteId() === inviteId) {
      setGeneratedInvite("");
      setCurrentInviteId(null);
    }
  }

  async function handleCopy(): Promise<void> {
    try {
      await navigator.clipboard.writeText(generatedInvite());
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      setError("Failed to copy to clipboard");
    }
  }

  async function handleInviteSubmit(e: Event): Promise<void> {
    e.preventDefault();
    const invite = inviteString().trim();
    if (!invite) return;
    setError(null);
    const err = await handleAddFriendFromInvite(invite);
    if (err) {
      setError(err);
    } else {
      props.onClose();
    }
  }

  const activeInvites = () =>
    friendsState.outgoingInvites.filter(
      (inv) => inv.inviteId !== currentInviteId(),
    );

  return (
    <div class="add-friend-section">
      <div class="add-friend-generate">
        <button
          class="form-btn-primary"
          onClick={handleGenerate}
          disabled={generating()}
        >
          {generating() ? "Generating..." : "Generate My Invite"}
        </button>
        <Show when={generatedInvite()}>
          <div class="add-friend-invite-result">
            <textarea
              class="add-friend-invite-text"
              readOnly
              value={generatedInvite()}
              rows={3}
            />
            <div class="add-friend-invite-actions">
              <button class="add-friend-copy-btn" onClick={handleCopy}>
                {copied() ? "Copied!" : "Copy"}
              </button>
              <button class="add-friend-cancel-btn" onClick={handleCancelCurrentInvite}>
                Cancel Invite
              </button>
            </div>
          </div>
        </Show>
      </div>

      <Show when={activeInvites().length > 0}>
        <div class="add-friend-invite-list">
          <div class="add-friend-invite-list-header">Active Invites</div>
          <For each={activeInvites()}>
            {(invite) => (
              <div class="add-friend-invite-item">
                <span
                  class="invite-status-badge"
                  classList={{ responded: invite.status === "accepted" }}
                >
                  {invite.status === "accepted" ? "Responded" : "Waiting"}
                </span>
                <Show when={invite.url}>
                  <span
                    class="invite-url-masked"
                    title="Click to copy invite URL"
                    onClick={() => navigator.clipboard.writeText(invite.url)}
                  >
                    {maskInviteUrl(invite.url)}
                  </span>
                </Show>
                <span class="invite-time">
                  {formatRelativeTime(invite.createdAt)}
                </span>
                <span class="invite-time">
                  expires in {formatTimeUntil(invite.expiresAt)}
                </span>
                <Show when={invite.url}>
                  <button
                    class="invite-copy-btn"
                    onClick={() => navigator.clipboard.writeText(invite.url)}
                  >
                    Copy
                  </button>
                </Show>
                <button
                  class="invite-cancel-btn"
                  onClick={() => handleCancelListInvite(invite.inviteId)}
                >
                  Cancel
                </button>
              </div>
            )}
          </For>
        </div>
      </Show>

      <div class="add-friend-divider">or paste a friend's invite</div>

      <form class="form-group" onSubmit={handleInviteSubmit}>
        <input
          class="form-input"
          type="text"
          placeholder="Paste invite link (rekindle://...)"
          value={inviteString()}
          onInput={(e) => setInviteString(e.currentTarget.value)}
        />
        <Show when={error()}>
          <div class="form-error">{error()}</div>
        </Show>
        <button class="form-btn-primary" type="submit" disabled={!inviteString().trim()}>
          Add from Invite
        </button>
      </form>
    </div>
  );
};

export default InviteLinkTab;
