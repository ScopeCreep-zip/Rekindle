import type { UnlistenFn } from "@tauri-apps/api/event";
import { subscribeChatEvents, subscribeGroupDmEvents } from "../ipc/channels";
import { isLegacy, type GroupDmEvent, type GroupDmMember } from "../ipc/channels/chat_events";
import type { GroupDmMemberView, SendState } from "../ipc/commands";
import { groupDmState, setGroupDmState } from "../stores/group-dm.store";
import { handleLoadGroupDms } from "../actions/group-dm.actions";

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
    });
    return;
  }
  if ("joined" in event || "left" in event || "keyRotated" in event) {
    void handleLoadGroupDms();
  }
}

/** The buddy list: invites and roster changes of every group. */
export function subscribeBuddyListGroupDmEvents(): Promise<UnlistenFn> {
  return subscribeGroupDmEvents(applyListEvent);
}

function applySendStatus(groupId: string, clientId: string, state: SendState): void {
  const convo = groupDmState.conversations[groupId];
  if (!convo) return;
  const idx = convo.messages.findIndex((m) => m.serverMessageId === clientId);
  if (idx < 0) return;
  setGroupDmState("conversations", groupId, "messages", idx, "status", state);
}

function setTyping(groupId: string, who: string, typing: boolean): void {
  if (!groupDmState.conversations[groupId]) {
    setGroupDmState("conversations", groupId, { messages: [], typing: {} });
  }
  setGroupDmState("conversations", groupId, "typing", who, typing);
}

/** One group's chat window: its roster, delivery states and typing. */
export function subscribeGroupChatEvents(groupId: string): Promise<UnlistenFn>[] {
  const lifecycle = subscribeGroupDmEvents((event) => {
    applyListEvent(event);
    if ("sendStatus" in event && event.sendStatus.groupId === groupId) {
      applySendStatus(groupId, event.sendStatus.clientId, event.sendStatus.state);
    }
  });
  const typing = subscribeChatEvents((event) => {
    if (isLegacy(event) || !("typing" in event)) return;
    const t = event.typing;
    const started = "started" in t;
    const inner = "started" in t ? t.started : t.stopped;
    if ("groupDm" in inner.context && inner.context.groupDm.groupId === groupId) {
      setTyping(groupId, inner.who, started);
    }
  });
  return [lifecycle, typing];
}
