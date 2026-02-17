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
  const [editingStatus, setEditingStatus] = createSignal(false);
  const [statusEditValue, setStatusEditValue] = createSignal("");

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

  function startEditingStatus(): void {
    setStatusEditValue(authState.statusMessage ?? "");
    setEditingStatus(true);
  }

  async function commitStatusEdit(): Promise<void> {
    const msg = statusEditValue().trim();
    const oldMsg = authState.statusMessage;
    if (msg !== (oldMsg ?? "")) {
      setAuthState("statusMessage", msg || null);
      try {
        await commands.setStatusMessage(msg);
      } catch (e) {
        console.error("Failed to set status message:", e);
        setAuthState("statusMessage", oldMsg);
      }
    }
    setEditingStatus(false);
  }

  function handleStatusKeyDown(e: KeyboardEvent): void {
    if (e.key === "Enter") {
      commitStatusEdit();
    } else if (e.key === "Escape") {
      setEditingStatus(false);
    }
  }

  return (
    <div class="identity-bar">
      <div class="identity-bar-top-row">
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
      <Show when={!editingStatus()} fallback={
        <input
          class="identity-bar-status-msg-edit"
          value={statusEditValue()}
          onInput={(e: InputEvent) => setStatusEditValue((e.target as HTMLInputElement).value)}
          onKeyDown={handleStatusKeyDown}
          onBlur={commitStatusEdit}
          placeholder="Set a status message..."
          autofocus
        />
      }>
        <div
          class="identity-bar-status-msg"
          onClick={startEditingStatus}
          title="Click to set status message"
        >
          {authState.statusMessage || "Set a status message..."}
        </div>
      </Show>
    </div>
  );
};

export default UserIdentityBar;
