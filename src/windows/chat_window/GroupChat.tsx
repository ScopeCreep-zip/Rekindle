import { Component, For, Show, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { startEventStream } from "../../ipc/channels";
import OwnRecordsWait from "../../components/common/OwnRecordsWait";
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
  handleRenameGroupDm,
  handleRetryGroupSend,
  handleSendGroupDm,
  handleLoadGroupDms,
  handleLoadGroupMessages,
  handleGroupDisplayed,
} from "../../actions/group-dm.actions";
import { handleLoadDisappearingTimer } from "../../actions/disappearing.actions";
import DisappearingTimerControl from "../../components/chat/DisappearingTimerControl";
import InviteToRoomModal from "../../components/chat/InviteToRoomModal";
import GameLobbyCard from "../../components/chat/GameLobbyCard";
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
    return Object.values(typing).filter((name): name is string => !!name);
  });

  // Stored history is in: from here on, new messages are announced (§13b).
  const [historyReady, setHistoryReady] = createSignal(false);
  const unlisteners: Promise<UnlistenFn>[] = [];
  // Focusing the window displays what it shows: received disappearing
  // timers start (plan E7.1).
  const onFocus = (): void => handleGroupDisplayed(groupId);
  onMount(async () => {
    unlisteners.push(
      ...subscribeGroupChatEvents(groupId, () => authState.publicKey ?? "", onFocus),
    );
    void startEventStream(() => void hydrateState());
    await hydrateState();
    await handleLoadGroupDms();
    await Promise.all([handleLoadGroupMessages(groupId, 50), handleLoadDisappearingTimer(groupId)]);
    setHistoryReady(true);
    window.addEventListener("focus", onFocus);
  });
  onCleanup(() => {
    window.removeEventListener("focus", onFocus);
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

  // Rename and Invite (plan E7.2 G7/G8) are always listed; the Host's
  // verdict (`renameRefused`, `inviteRefused`) disables each with its reason.
  const renameRefused = (): string | null => group()?.renameRefused ?? "This group is not in your list.";
  const inviteRefused = (): string | null => group()?.inviteRefused ?? "This group is not in your list.";
  const [renaming, setRenaming] = createSignal<string | null>(null);
  const [inviting, setInviting] = createSignal(false);
  const [roomError, setRoomError] = createSignal<string | null>(null);

  async function rename(e: Event): Promise<void> {
    e.preventDefault();
    const next = renaming()?.trim() ?? "";
    if (!next) return;
    const err = await handleRenameGroupDm(groupId, next);
    setRoomError(err);
    if (!err) setRenaming(null);
  }

  return (
    <>
      <Titlebar title={`Chat — ${name()}`} showMaximize />
      <div class="group-chat-header">
        <ul
          class="group-chat-members"
          aria-label={`Members of ${name()}`}
          title={members()
            .map((m) => m.label)
            .join(", ")}
        >
          <For each={members()}>{(m) => <li class="group-chat-member">{m.label}</li>}</For>
        </ul>
        <Show when={held()}>
          <DisappearingTimerControl conversation={groupId} label={name()} />
        </Show>
        <Show
          when={renaming() !== null}
          fallback={
            <button
              class="form-btn-secondary group-chat-action"
              aria-disabled={renameRefused() !== null ? "true" : undefined}
              aria-describedby={renameRefused() !== null ? "group-rename-reason" : undefined}
              title={renameRefused() ?? `Rename ${name()}`}
              onClick={() => renameRefused() === null && setRenaming(name())}
            >
              Rename
            </button>
          }
        >
          <form class="group-chat-rename" onSubmit={(e) => void rename(e)}>
            <input
              class="form-input group-chat-rename-input"
              aria-label={`New name for ${name()}`}
              value={renaming() ?? ""}
              onInput={(e) => setRenaming(e.currentTarget.value)}
              onKeyDown={(e) => e.key === "Escape" && setRenaming(null)}
              ref={(el) => queueMicrotask(() => el.select())}
            />
          </form>
        </Show>
        <button
          class="form-btn-secondary group-chat-action"
          aria-disabled={inviteRefused() !== null ? "true" : undefined}
          aria-describedby={inviteRefused() !== null ? "group-invite-reason" : undefined}
          title={inviteRefused() ?? `Invite friends to ${name()}`}
          onClick={() => inviteRefused() === null && setInviting(true)}
        >
          Invite…
        </button>
        <span class="sr-only" id="group-rename-reason">{renameRefused()}</span>
        <span class="sr-only" id="group-invite-reason">{inviteRefused()}</span>
        <button
          class="form-btn-secondary group-chat-leave"
          aria-label={`Leave ${name()}`}
          onClick={() => void leave()}
        >
          Leave
        </button>
      </div>
      <Show when={roomError()}>
        {(message) => <div class="form-error group-chat-error" role="alert">{message()}</div>}
      </Show>
      <InviteToRoomModal groupId={inviting() ? groupId : null} onClose={() => setInviting(false)} />
      <Show when={held()}>
        <GameLobbyCard groupId={groupId} memberNames={memberNames()} />
      </Show>
      <div id="main-content" tabindex="-1" class="window-main">
        <MessageList
          messages={messages()}
          ownName={authState.displayName ?? "You"}
          peerName={name()}
          memberNames={memberNames()}
          onRetry={(id) => void handleRetryGroupSend(groupId, id)}
          onDiscard={(id) => void handleDiscardGroupSend(groupId, id)}
          historyReady={historyReady()}
        />
        <TypingIndicator isTyping={typingNames().length > 0} peerName={typingNames().join(", ")} />
        {/* A group send waits until our identity is published (plan E2.5
         *  FC3); the composer says so while it is not. */}
        <OwnRecordsWait active={true} />
        <MessageInput
          label={`Message ${name()}`}
          peerId={groupId}
          onSend={(_, body) => void send(body)}
          sendButton
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
