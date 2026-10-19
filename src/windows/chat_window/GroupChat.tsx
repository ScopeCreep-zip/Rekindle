import { Component, createMemo, onCleanup, onMount } from "solid-js";
import type { UnlistenFn } from "@tauri-apps/api/event";
import Titlebar from "../../components/titlebar/Titlebar";
import MessageList from "../../components/chat/MessageList";
import MessageInput from "../../components/chat/MessageInput";
import { authState } from "../../stores/auth.store";
import { dmState, setDmState } from "../../stores/dm.store";
import { addToast } from "../../stores/toast.store";
import { subscribeDmInbox } from "../../handlers/dm.handlers";
import { startEventStream } from "../../ipc/channels";
import { commands } from "../../ipc/commands";
import type { Message } from "../../stores/chat.store";

/// The chat window's group mode: a group DM over its SMPL record
/// (architecture §27.2), rendered from the group rows of `list_dms`,
/// `get_dm_messages` and `send_dm_message` until E2.6's group DMs
/// replace them. Group calls are E4.2's, so it has no call controls.
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

  /// The group list and this group's scrollback from the backend: at
  /// mount, and again when the event stream says this window may be stale.
  async function load(): Promise<void> {
    const list = await commands.listDms();
    const map: Record<string, (typeof list)[number]> = {};
    for (const conv of list) {
      if (conv.isGroup) map[conv.recordKey] = conv;
    }
    setDmState("conversations", map);
    const rows = await commands.getDmMessages(props.recordKey, 200);
    const me = ownPublicKey();
    setDmState(
      "messages",
      props.recordKey,
      rows.map((row, idx) => ({
        id: idx + 1,
        senderId: row.senderPseudonym,
        body: row.body,
        timestamp: row.timestamp * 1000,
        isOwn: row.senderPseudonym === me,
      })),
    );
  }

  onMount(async () => {
    unlisteners.push(subscribeDmInbox(() => ownPublicKey()));
    void startEventStream(() => void load());
    await load();
  });

  onCleanup(() => {
    for (const p of unlisteners) p.then((u) => u());
  });

  /// The backend echoes our own message as a received one, so the log
  /// has one source of order; a refused send says why.
  async function onSend(_id: string, body: string): Promise<void> {
    const trimmed = body.trim();
    if (!trimmed) return;
    try {
      await commands.sendDmMessage(props.recordKey, trimmed);
    } catch (e) {
      addToast(`Message not sent: ${String(e)}`, "error");
    }
  }

  return (
    <>
      <Titlebar title={`Chat — ${groupName()}`} showMaximize />
      <div id="main-content" tabindex="-1" class="window-main">
        <MessageList messages={messages()} ownName={ownName()} peerName={groupName()} />
        <MessageInput peerId={props.recordKey} onSend={onSend} />
      </div>
    </>
  );
};

export default GroupChat;
