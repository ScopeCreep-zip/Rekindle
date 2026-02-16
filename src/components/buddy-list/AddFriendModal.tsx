import { Component, Show, createSignal } from "solid-js";
import Modal from "../common/Modal";
import { friendsState, setFriendsState } from "../../stores/friends.store";
import { handleAddFriend, handleGenerateInvite, handleAddFriendFromInvite } from "../../handlers/buddy.handlers";

type Tab = "invite" | "key";

const AddFriendModal: Component = () => {
  const [tab, setTab] = createSignal<Tab>("invite");
  const [inviteString, setInviteString] = createSignal("");
  const [generatedInvite, setGeneratedInvite] = createSignal("");
  const [publicKey, setPublicKey] = createSignal("");
  const [message, setMessage] = createSignal("Hey, add me!");
  const [error, setError] = createSignal<string | null>(null);
  const [generating, setGenerating] = createSignal(false);
  const [copied, setCopied] = createSignal(false);

  function handleClose(): void {
    setFriendsState("showAddFriend", false);
    setInviteString("");
    setGeneratedInvite("");
    setPublicKey("");
    setMessage("Hey, add me!");
    setError(null);
    setGenerating(false);
    setCopied(false);
  }

  async function handleGenerate(): Promise<void> {
    setGenerating(true);
    setError(null);
    const invite = await handleGenerateInvite();
    setGenerating(false);
    if (invite) {
      setGeneratedInvite(invite);
    } else {
      setError("Failed to generate invite link");
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
                <button class="add-friend-copy-btn" onClick={handleCopy}>
                  {copied() ? "Copied!" : "Copy"}
                </button>
              </div>
            </Show>
          </div>

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
