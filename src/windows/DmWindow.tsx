import { Component, createMemo, createSignal, onCleanup, onMount, Show } from "solid-js";
import type { UnlistenFn } from "@tauri-apps/api/event";
import Titlebar from "../components/titlebar/Titlebar";
import MessageList from "../components/chat/MessageList";
import MessageInput from "../components/chat/MessageInput";
import { authState } from "../stores/auth.store";
import { friendsState } from "../stores/friends.store";
import { dmState, setDmState } from "../stores/dm.store";
import { handleSendDm, subscribeDmInbox, handleListDms } from "../handlers/dm.handlers";
import { handleStartDmCall } from "../handlers/calls.handlers";
import { commands } from "../ipc/commands";
import type { Message } from "../stores/chat.store";

function getRecordKeyFromUrl(): string {
  const params = new URLSearchParams(window.location.search);
  return params.get("record") ?? "";
}

const DmWindow: Component = () => {
  const recordKey = getRecordKeyFromUrl();
  const ownPublicKey = createMemo(() => authState.publicKey ?? "");
  const ownName = createMemo(() => authState.displayName ?? "You");

  const conversation = createMemo(() => dmState.conversations[recordKey]);

  /// Friendly label for the window: prefer the initiator pseudonym from
  /// the persisted DM row, then a friend display name if the other
  /// participant is in our friends list, then the truncated record key.
  const peerName = createMemo(() => {
    const conv = conversation();
    if (!conv) return recordKey.slice(0, 12) + "…";
    if (!conv.isGroup) {
      const peer = conv.participants.find((p) => p.publicKey !== ownPublicKey());
      if (peer) {
        const friend = friendsState.friends[peer.publicKey];
        if (friend?.displayName) return friend.displayName;
      }
      return conv.initiatorPseudonym;
    }
    return conv.initiatorPseudonym + " (group)";
  });

  const messages = createMemo<Message[]>(() => dmState.messages[recordKey] ?? []);

  let unlisten: Promise<UnlistenFn> | undefined;

  onMount(async () => {
    setDmState("activeRecordKey", recordKey);
    unlisten = subscribeDmInbox(() => ownPublicKey());
    // Hydrate conversation list (we may have been launched directly into
    // the DM window without ever populating the buddy list's dm map).
    await handleListDms();
    // Pull message scrollback from SQLite into the in-memory list.
    const rows = await commands.getDmMessages(recordKey, 200);
    const me = ownPublicKey();
    const hydrated: Message[] = rows.map((row, idx) => ({
      id: idx + 1,
      senderId: row.senderPseudonym,
      body: row.body,
      timestamp: row.timestamp * 1000,
      isOwn: row.senderPseudonym === me,
    }));
    setDmState("messages", recordKey, hydrated);
  });

  onCleanup(() => {
    unlisten?.then((u) => u());
  });

  async function onSend(_id: string, body: string): Promise<void> {
    const trimmed = body.trim();
    if (!trimmed) return;
    await handleSendDm(recordKey, trimmed);
  }

  /// Plan §Failure 5 — start a 1:1 call. Group DMs aren't supported by
  /// the rekindle-calls signalling crate yet (architecture §10.10
  /// reserves group calls for the per-participant X25519 wrap path).
  function dmPeerKey(): string | null {
    const conv = conversation();
    if (!conv || conv.isGroup) return null;
    const peer = conv.participants.find((p) => p.publicKey !== ownPublicKey());
    return peer?.publicKey ?? null;
  }

  async function startCall(video: boolean): Promise<void> {
    const peer = dmPeerKey();
    if (!peer) return;
    await handleStartDmCall(peer, peerName(), video);
  }

  return (
    <div class="app-frame">
      {/* Architecture §32 a11y — keyboard skip link past titlebar. */}
      <a href="#main-content" class="skip-link">Skip to messages</a>
      <Titlebar title={`DM — ${peerName()}`} showMaximize />
      <Show
        when={recordKey}
        fallback={
          <div class="empty-placeholder">
            <div class="empty-placeholder-title">No DM selected</div>
            <div class="empty-placeholder-subtitle">
              Open a DM from the buddy list to start chatting.
            </div>
          </div>
        }
      >
        <div id="main-content" tabindex="-1" class="window-main">
          {/* Plan §Failure 5 — direct-call quick actions. Hidden for
           *  group DMs because the offer/accept signalling is 1:1
           *  only at this stage. */}
          <Show when={dmPeerKey()}>
            <div class="dm-call-bar">
              <button
                type="button"
                class="form-btn-secondary"
                onClick={() => void startCall(false)}
              >
                Voice call
              </button>
              <button
                type="button"
                class="form-btn-secondary"
                onClick={() => void startCall(true)}
              >
                Video call
              </button>
            </div>
          </Show>
          <MessageList
            messages={messages()}
            ownName={ownName()}
            peerName={peerName()}
          />
          <MessageInput peerId={recordKey} onSend={onSend} />
        </div>
      </Show>
    </div>
  );
};

export default DmWindow;
