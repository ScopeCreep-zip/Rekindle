import { Component, Show, For, createSignal, createEffect, on } from "solid-js";
import Modal from "../common/Modal";
import { friendsState, setFriendsState } from "../../stores/friends.store";
import {
  handleAddFriend,
  handleGenerateInvite,
  handleAddFriendFromInvite,
  handleCancelInvite,
  handleLoadOutgoingInvites,
} from "../../handlers/buddy.handlers";
import { maskInviteUrl } from "../../utils/masking";

type Tab = "invite" | "key";

function formatRelativeTime(epochMs: number): string {
  const now = Date.now();
  const diffMs = Math.abs(now - epochMs);
  const minutes = Math.floor(diffMs / 60000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}

function formatTimeUntil(epochMs: number): string {
  const now = Date.now();
  const diffMs = epochMs - now;
  if (diffMs <= 0) return "expired";
  const minutes = Math.floor(diffMs / 60000);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h`;
  const days = Math.floor(hours / 24);
  return `${days}d`;
}

const AddFriendModal: Component = () => {
  const [tab, setTab] = createSignal<Tab>("invite");
  const [inviteString, setInviteString] = createSignal("");
  const [generatedInvite, setGeneratedInvite] = createSignal("");
  const [currentInviteId, setCurrentInviteId] = createSignal<string | null>(null);
  const [publicKey, setPublicKey] = createSignal("");
  const [message, setMessage] = createSignal("Hey, add me!");
  const [error, setError] = createSignal<string | null>(null);
  const [generating, setGenerating] = createSignal(false);
  const [copied, setCopied] = createSignal(false);

  // Load outgoing invites when modal opens on invite tab
  createEffect(
    on(
      () => friendsState.showAddFriend,
      (isOpen) => {
        if (isOpen && tab() === "invite") {
          handleLoadOutgoingInvites();
        }
      },
    ),
  );

  // Also load when switching to invite tab while modal is open
  createEffect(
    on(tab, (currentTab) => {
      if (friendsState.showAddFriend && currentTab === "invite") {
        handleLoadOutgoingInvites();
      }
    }),
  );

  function handleClose(): void {
    setFriendsState("showAddFriend", false);
    setInviteString("");
    setGeneratedInvite("");
    setCurrentInviteId(null);
    setPublicKey("");
    setMessage("Hey, add me!");
    setError(null);
    setGenerating(false);
    setCopied(false);
  }

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
    // If this was the currently displayed invite, clear it
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
      handleClose();
    }
  }

  async function handleKeySubmit(e: Event): Promise<void> {
    e.preventDefault();
    const key = publicKey().trim();
    if (!key) return;
    setError(null);
    const err = await handleAddFriend(key, message().trim());
    if (err) {
      setError(err);
    } else {
      handleClose();
    }
  }

  // Filter out the currently-displayed invite from the list to avoid duplication
  const activeInvites = () =>
    friendsState.outgoingInvites.filter(
      (inv) => inv.inviteId !== currentInviteId(),
    );

  return (
    <Modal
      isOpen={friendsState.showAddFriend}
      title="Add Friend"
      onClose={handleClose}
    >
      <div class="add-friend-tabs">
        <button
          class="add-friend-tab"
          classList={{ active: tab() === "invite" }}
          onClick={() => { setTab("invite"); setError(null); }}
        >
          Invite Link
        </button>
        <button
          class="add-friend-tab"
          classList={{ active: tab() === "key" }}
          onClick={() => { setTab("key"); setError(null); }}
        >
          Public Key
        </button>
      </div>

      <Show when={tab() === "invite"}>
        <div class="add-friend-section">
          <div class="add-friend-generate">
            <button
              class="add-friend-btn"
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

          <form class="add-friend-form" onSubmit={handleInviteSubmit}>
            <input
              class="add-friend-input"
              type="text"
              placeholder="Paste invite link (rekindle://...)"
              value={inviteString()}
              onInput={(e) => setInviteString(e.currentTarget.value)}
            />
            <Show when={error()}>
              <div class="login-error">{error()}</div>
            </Show>
            <button class="add-friend-btn" type="submit" disabled={!inviteString().trim()}>
              Add from Invite
            </button>
          </form>
        </div>
      </Show>

      <Show when={tab() === "key"}>
        <form class="add-friend-form" onSubmit={handleKeySubmit}>
          <input
            class="add-friend-input"
            type="text"
            placeholder="Enter public key..."
            value={publicKey()}
            onInput={(e) => setPublicKey(e.currentTarget.value)}
          />
          <input
            class="add-friend-input"
            type="text"
            placeholder="Message (optional)"
            value={message()}
            onInput={(e) => setMessage(e.currentTarget.value)}
          />
          <Show when={error()}>
            <div class="login-error">{error()}</div>
          </Show>
          <button class="add-friend-btn" type="submit" disabled={!publicKey().trim()}>
            Send Friend Request
          </button>
        </form>
      </Show>
    </Modal>
  );
};

export default AddFriendModal;
