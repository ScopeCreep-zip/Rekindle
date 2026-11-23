import type { UnlistenFn } from "@tauri-apps/api/event";
import { subscribeChatEvents, subscribeGroupDmEvents } from "../ipc/channels";
import {
  conversationId,
  type ChannelMessageEvent,
  type GroupDmEvent,
  type GroupDmMember,
} from "../ipc/channels/chat_events";
import { applyTimerChanged, withoutExpired } from "../actions/disappearing.actions";
import type { GroupDmMemberView, SendState } from "../ipc/commands";
import type { DeliveryRetry } from "../ipc/channels/chat_events";
import { groupDmState, setGroupDmState } from "../stores/group-dm.store";
import { handleLoadGroupDms } from "../actions/group-dm.actions";
import { transformDmMessage, transformHistoryGap } from "../utils/transformers";

// Group-DM events (plan E2.6). The Host decides every change; an invite
// arrives whole and is shown as is, and a roster change (join, leave, key
// rotation) re-reads the Host's list instead of editing it here.

function memberView(m: GroupDmMember): GroupDmMemberView {
  return { key: m.publicKey, label: m.displayName };
}

function applyListEvent(event: GroupDmEvent): void {
  if ("invited" in event) {
    const i = event.invited;
    setGroupDmState("groups", i.groupId, {
      groupId: i.groupId,
      name: i.name,
      members: i.members.map(memberView),
      epoch: 0,
      pending: true,
      originPeer: null,
      favourite: false,
      renameRefused: "accept the invite first",
      inviteRefused: "accept the invite first",
    });
    return;
  }
  if ("joined" in event || "left" in event || "keyRotated" in event) {
    void handleLoadGroupDms();
  }
  if ("renamed" in event) {
    // The Host accepted the creator's rename (plan E7.2 G7): its name, as sent.
    const { groupId, name } = event.renamed;
    if (groupDmState.groups[groupId]) setGroupDmState("groups", groupId, "name", name);
  }
}

/** The buddy list: invites and roster changes of every group. */
export function subscribeBuddyListGroupDmEvents(): Promise<UnlistenFn> {
  return subscribeGroupDmEvents(applyListEvent);
}

function applySendStatus(groupId: string, clientId: string, state: SendState, retry: DeliveryRetry | null): void {
  const convo = groupDmState.conversations[groupId];
  if (!convo) return;
  const idx = convo.messages.findIndex((m) => m.serverMessageId === clientId);
  if (idx < 0) return;
  setGroupDmState("conversations", groupId, "messages", idx, "status", state);
  setGroupDmState("conversations", groupId, "messages", idx, "retry", retry);
}

// A live member message carries no row id: it takes a local negative one
// (below any history-gap marker's) until a reload reads the stored row.
let liveRows = 0;

function appendMessage(groupId: string, m: Extract<GroupDmEvent, { message: unknown }>["message"]): void {
  const convo = groupDmState.conversations[groupId];
  if (!convo) {
    setGroupDmState("conversations", groupId, { messages: [], typing: {} });
  } else if (convo.messages.some((x) => x.serverMessageId === m.messageId)) {
    return;
  }
  liveRows += 1;
  const row = transformDmMessage({
    id: -1_000_000_000 - liveRows,
    messageId: m.messageId,
    senderId: m.sender.publicKey,
    body: m.body,
    timestamp: m.timestamp,
    isOwn: false,
    sendState: "delivered",
    historyGapThrough: null,
    expireSeconds: 0,
    expireAt: null,
    timerNoticeSeconds: null,
  });
  // The live copy's timer is unknown until its row is read.
  row.expireSeconds = undefined;
  row.expireAt = undefined;
  setGroupDmState("conversations", groupId, "messages", (rows) => [...rows, row]);
  setGroupDmState("conversations", groupId, "typing", m.sender.publicKey, null);
}

// A member's chain retired entries we never read (`historyGap`): the
// backend stored the marker row, appended here as history shows it (the
// DM marker's shape and rendering), once per member and `throughSeq`.
function applyHistoryGap(groupId: string, member: string, throughSeq: number): void {
  const marker = transformHistoryGap({ peerKey: member, throughSeq });
  const convo = groupDmState.conversations[groupId];
  if (!convo) {
    setGroupDmState("conversations", groupId, { messages: [marker], typing: {} });
    return;
  }
  const held = convo.messages.some(
    (m) => m.senderId === member && m.historyGapThrough === throughSeq,
  );
  if (held) return;
  setGroupDmState("conversations", groupId, "messages", (msgs) => [...msgs, marker]);
}

function setTyping(groupId: string, who: string, name: string | null): void {
  if (!groupDmState.conversations[groupId]) {
    setGroupDmState("conversations", groupId, { messages: [], typing: {} });
  }
  setGroupDmState("conversations", groupId, "typing", who, name);
}

// The group's disappearing timer changed (its notice joins the timeline),
// or the Host deleted expired messages (plan E7.1).
function applyDisappearing(groupId: string, event: ChannelMessageEvent, ownKey: () => string): void {
  if ("disappearingTimerChanged" in event) {
    const change = event.disappearingTimerChanged;
    if (conversationId(change.context) !== groupId) return;
    const notice = applyTimerChanged(change, ownKey());
    if (!groupDmState.conversations[groupId]) {
      setGroupDmState("conversations", groupId, { messages: [], typing: {} });
    }
    setGroupDmState("conversations", groupId, "messages", (rows) => [...rows, notice]);
    return;
  }
  if ("messagesExpired" in event) {
    const gone = event.messagesExpired;
    if (conversationId(gone.context) !== groupId || !groupDmState.conversations[groupId]) return;
    setGroupDmState("conversations", groupId, "messages", (rows) => withoutExpired(rows, gone.clientIds));
  }
}

/** One group's chat window: its roster, members' messages, our sends'
 *  states and typing. */
export function subscribeGroupChatEvents(
  groupId: string,
  ownKey: () => string,
  onDisplayed: () => void,
): Promise<UnlistenFn>[] {
  const lifecycle = subscribeGroupDmEvents((event) => {
    applyListEvent(event);
    if ("sendStatus" in event && event.sendStatus.groupId === groupId) {
      applySendStatus(groupId, event.sendStatus.clientId, event.sendStatus.state, event.sendStatus.retry);
    }
    if ("message" in event && event.message.groupId === groupId) {
      appendMessage(groupId, event.message);
      onDisplayed();
      return;
    }
    if ("historyGap" in event && event.historyGap.groupId === groupId) {
      const gap = event.historyGap;
      applyHistoryGap(groupId, gap.member.publicKey, gap.throughSeq);
    }
  });
  const typing = subscribeChatEvents((event) => {
    if ("channelMessage" in event) {
      applyDisappearing(groupId, event.channelMessage, ownKey);
      return;
    }
    if (!("typing" in event)) return;
    const t = event.typing;
    const inner = "started" in t ? t.started : t.stopped;
    if ("groupDm" in inner.context && inner.context.groupDm.groupId === groupId) {
      // The typist's name is the Host's (plan E3.4 S9b), never the window's.
      setTyping(groupId, inner.who, "started" in t ? t.started.name : null);
    }
  });
  return [lifecycle, typing];
}
