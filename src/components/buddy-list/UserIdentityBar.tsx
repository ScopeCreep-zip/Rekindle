import { Component, createSignal, Show } from "solid-js";
import { authState, setAuthState } from "../../stores/auth.store";
import { handleCopyPublicKey } from "../../handlers/buddy.handlers";
import { commands } from "../../ipc/commands";
import StatusDot from "../status/StatusDot";
import Avatar from "../common/Avatar";
import NotificationCenter from "./NotificationCenter";

const UserIdentityBar: Component = () => {
  const [editing, setEditing] = createSignal(false);
  const [editValue, setEditValue] = createSignal("");

  function truncatedKey(): string {
    const key = authState.publicKey;
    if (!key) return "";
    return key.slice(0, 8) + "..." + key.slice(-6);
  }

  function startEditing(): void {
    setEditValue(authState.displayName ?? "");
    setEditing(true);
  }

  async function commitEdit(): Promise<void> {
    const name = editValue().trim();
    if (name && name !== authState.displayName) {
      const oldName = authState.displayName;
      setAuthState("displayName", name);
      try {
        await commands.setNickname(name);
      } catch (e) {
        console.error("Failed to set nickname:", e);
        // Revert on failure
        setAuthState("displayName", oldName);
      }
    }
    setEditing(false);
  }

  function handleEditKeyDown(e: KeyboardEvent): void {
    if (e.key === "Enter") {
      commitEdit();
    } else if (e.key === "Escape") {
      setEditing(false);
    }
  }

  return (
    <div class="identity-bar">
      <Avatar
        displayName={authState.displayName ?? "?"}
        size={24}
        avatarUrl={authState.avatarUrl ?? undefined}
      />
      <StatusDot status={authState.status} />
      <Show when={!editing()} fallback={
        <input
          class="identity-bar-edit"
          value={editValue()}
          onInput={(e: InputEvent) => setEditValue((e.target as HTMLInputElement).value)}
          onKeyDown={handleEditKeyDown}
          onBlur={commitEdit}
          autofocus
        />
      }>
        <span class="identity-bar-name" onClick={startEditing} title="Click to edit display name">
          {authState.displayName ?? "Unknown"}
        </span>
      </Show>
      <span class="identity-bar-key" onClick={handleCopyPublicKey} title="Click to copy public key">
        {truncatedKey()}
      </span>
      <Show when={authState.gameInfo}>
        {(game) => (
          <span class="identity-bar-game" title={`Playing ${game().gameName}`}>
            {game().gameName}
          </span>
        )}
      </Show>
      <NotificationCenter />
    </div>
  );
};

export default UserIdentityBar;
