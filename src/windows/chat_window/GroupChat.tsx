import { Component, createMemo, onCleanup, onMount } from "solid-js";
import type { UnlistenFn } from "@tauri-apps/api/event";
import Titlebar from "../../components/titlebar/Titlebar";
import MessageList from "../../components/chat/MessageList";
import { authState } from "../../stores/auth.store";
import { dmState } from "../../stores/dm.store";
import { subscribeDmInbox } from "../../handlers/dm.handlers";
import { startEventStream } from "../../ipc/channels";
import { loadGroupChat } from "../../actions/group_dm.actions";
import type { Message } from "../../stores/chat.store";

/// The chat window's group mode: a group DM over its SMPL record
/// (architecture §27.2), rendered from the group rows of `list_dms` and
/// `get_dm_messages` until E2.6's group DMs replace them. Its input is
/// off until then (one notice, not a toast per send). Group calls are
/// E4.2's, so it has no call controls.
const GroupChat: Component<{ recordKey: string }> = (props) => {
  const ownPublicKey = createMemo(() => authState.publicKey ?? "");
  const ownName = createMemo(() => authState.displayName ?? "You");
  const conversation = createMemo(() => dmState.conversations[props.recordKey]);
  const groupName = createMemo(() => {
    const conv = conversation();
    return conv ? `${conv.initiatorPseudonym} (group)` : "Group DM";
  });
  const messages = createMemo<Message[]>(() => dmState.messages[props.recordKey] ?? []);

  const unlisteners: Promise<UnlistenFn>[] = [];

  /// The group from the backend: at mount, and again when the event
  /// stream says this window may be stale.
  const load = (): Promise<void> => loadGroupChat(props.recordKey, ownPublicKey());

  onMount(async () => {
    unlisteners.push(subscribeDmInbox());
    void startEventStream(() => void load());
    await load();
  });

  onCleanup(() => {
    for (const p of unlisteners) p.then((u) => u());
  });

  return (
    <>
      <Titlebar title={`Chat — ${groupName()}`} showMaximize />
      <div id="main-content" tabindex="-1" class="window-main">
        <MessageList messages={messages()} ownName={ownName()} peerName={groupName()} />
        {/* Sending waits for E2.6's group DMs; the log stays readable. */}
        <div class="typing-indicator" role="status">
          <span class="typing-label">Sending to group chats is not available yet.</span>
        </div>
      </div>
    </>
  );
};

export default GroupChat;
