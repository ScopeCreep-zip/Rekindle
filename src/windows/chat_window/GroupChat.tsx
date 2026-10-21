import { Component, For, Show, createMemo, onCleanup, onMount } from "solid-js";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { startEventStream } from "../../ipc/channels";
import Titlebar from "../../components/titlebar/Titlebar";
import MessageList from "../../components/chat/MessageList";
import TypingIndicator from "../../components/chat/TypingIndicator";
import MessageInput from "../../components/chat/MessageInput";
import { authState } from "../../stores/auth.store";
import { groupDmState } from "../../stores/group-dm.store";
import { hydrateState } from "../../stores/hydrate";
import {
  handleDiscardGroupSend,
  handleGroupDraftChanged,
  handleLeaveGroupDm,
  handleRetryGroupSend,
  handleSendGroupDm,
  handleLoadGroupDms,
  handleLoadGroupMessages,
} from "../../actions/group-dm.actions";
import { subscribeGroupChatEvents } from "../../handlers/group-dm.handlers";

/// The chat window's group mode (plan E2.6): the group's stored history
/// (`group_dm_messages`) with members' messages as they arrive, its members
/// as the Host labels them, who is typing, our send (its echo following
/// `sendStatus`: queued, sent or failed, with Retry and Discard), our
/// typing reported per edit, and Leave (reviewed by the Host). `groupId` is 32 hex; the
/// backend builds the URL (`windows::open_chat_window`).
const GroupChat: Component<{ groupId: string }> = (props) => {
  const groupId = props.groupId;
  const group = createMemo(() => groupDmState.groups[groupId]);
  const name = createMemo(() => group()?.name ?? "Group DM");
  const members = createMemo(() => group()?.members ?? []);
  const memberNames = createMemo(() => {
    const names: Record<string, string> = {};
    for (const m of members()) names[m.key] = m.label;
    return names;
  });
  const messages = createMemo(() => groupDmState.conversations[groupId]?.messages ?? []);
  const typingNames = createMemo(() => {
    const typing = groupDmState.conversations[groupId]?.typing ?? {};
    return Object.keys(typing)
      .filter((k) => typing[k])
      .map((k) => memberNames()[k] ?? k.slice(0, 12));
  });

  const unlisteners: Promise<UnlistenFn>[] = [];
  onMount(async () => {
    unlisteners.push(...subscribeGroupChatEvents(groupId));
    void startEventStream(() => void hydrateState());
    await hydrateState();
    await handleLoadGroupDms();
    await handleLoadGroupMessages(groupId, 50);
  });
  onCleanup(() => {
    for (const p of unlisteners) p.then((unlisten) => unlisten());
  });

  const held = createMemo(() => !!group() && !group()!.pending);

  async function send(body: string): Promise<void> {
    const err = await handleSendGroupDm(groupId, body);
    if (err) console.error("group_dm_send failed:", err);
  }

  async function leave(): Promise<void> {
    const err = await handleLeaveGroupDm(groupId);
    if (!err) window.close();
  }

  return (
    <>
      <Titlebar title={`Chat — ${name()}`} showMaximize />
      <div class="group-chat-header">
        <ul class="group-chat-members" aria-label={`Members of ${name()}`}>
          <For each={members()}>{(m) => <li class="group-chat-member">{m.label}</li>}</For>
        </ul>
        <button
          class="group-chat-leave"
          aria-label={`Leave ${name()}`}
          onClick={() => void leave()}
        >
          Leave
        </button>
      </div>
      <div id="main-content" tabindex="-1" class="window-main">
        <MessageList
          messages={messages()}
          ownName={authState.displayName ?? "You"}
          peerName={name()}
          memberNames={memberNames()}
          onRetry={(id) => void handleRetryGroupSend(groupId, id)}
          onDiscard={(id) => void handleDiscardGroupSend(groupId, id)}
        />
        <TypingIndicator isTyping={typingNames().length > 0} peerName={typingNames().join(", ")} />
        <MessageInput
          peerId={groupId}
          onSend={(_, body) => void send(body)}
          onDraftChanged={(body) => handleGroupDraftChanged(groupId, body)}
          disabled={!held()}
          disabledMessage="You can send once you are in this group."
        />
        <Show when={!group()}>
          <div class="empty-placeholder-subtitle" role="status">
            This group is not in your list.
          </div>
        </Show>
      </div>
    </>
  );
};

export default GroupChat;
