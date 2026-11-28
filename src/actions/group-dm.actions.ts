import { reconcile } from "solid-js/store";
import { commands, type GroupDmView } from "../ipc/commands";
import { disappearingState } from "../stores/disappearing.store";
import { groupDmState, setGroupDmState } from "../stores/group-dm.store";
import { afterDraftReports, sendDraftReport } from "../utils/draft_reports";
import { errorMessage } from "../utils/error";
import { transformDmMessage } from "../utils/transformers";
import { awaitsExpiry } from "./disappearing.actions";

// Group DMs (plan E2.6). Each action calls the Host and renders what it
// answers; validation, the member caps and the review are the Host's.

/** Reload the groups and pending invites from the Host. */
export async function handleLoadGroupDms(): Promise<void> {
  try {
    const list = await commands.groupDmList();
    const byId: Record<string, GroupDmView> = {};
    for (const g of list) byId[g.groupId] = g;
    setGroupDmState("groups", reconcile(byId));
  } catch (e) {
    console.error("group_dm_list failed", e);
  }
}

/** Create a group; `null` on success, else the Host's error. */
export async function handleCreateGroupDm(name: string, members: string[]): Promise<string | null> {
  try {
    await commands.groupDmCreate(name, members);
    await handleLoadGroupDms();
    return null;
  } catch (e) {
    return errorMessage(e);
  }
}

async function answer(run: () => Promise<unknown>): Promise<string | null> {
  try {
    await run();
    await handleLoadGroupDms();
    return null;
  } catch (e) {
    return errorMessage(e);
  }
}

export const handleAcceptGroupDm = (groupId: string) =>
  answer(() => commands.groupDmAccept(groupId));
export const handleDeclineGroupDm = (groupId: string) =>
  answer(() => commands.groupDmDecline(groupId));
export const handleLeaveGroupDm = (groupId: string) => answer(() => commands.groupDmLeave(groupId));
/** Rename a group we created (plan E7.2 G7); the Host reviews it. */
export const handleRenameGroupDm = (groupId: string, name: string) =>
  answer(() => commands.groupDmRename(groupId, name));
/** Invite friends into a group we created (plan E7.2 G8); the Host reviews it. */
export const handleInviteToGroupDm = (groupId: string, members: string[]) =>
  answer(() => commands.groupDmInvite(groupId, members));

export function handleOpenGroupChat(groupId: string): void {
  void commands.openGroupChatWindow(groupId);
}

/** Load the group's newest `limit` stored messages. */
export async function handleLoadGroupMessages(groupId: string, limit: number): Promise<void> {
  if (await loadGroupRows(groupId, limit)) handleGroupDisplayed(groupId);
}

/** Read the stored rows (timer notices are stored rows too, plan E7.1). */
async function loadGroupRows(groupId: string, limit: number): Promise<boolean> {
  try {
    const rows = await commands.groupDmMessages(groupId, limit);
    const held = groupDmState.conversations[groupId];
    const mapped = rows.map(transformDmMessage);
    // Rows that arrived live after the history was read stay, as in a 1:1
    // conversation (chat.actions.ts handleLoadHistory, peer finding F-echo):
    // not in the snapshot and newer than its newest row.
    const newest = mapped.reduce((t, m) => Math.max(t, m.timestamp), 0);
    const stored = new Set(mapped.map((m) => m.serverMessageId).filter(Boolean));
    const live = (held?.messages ?? []).filter(
      (m) => m.serverMessageId && !stored.has(m.serverMessageId) && m.timestamp > newest
    );
    setGroupDmState("conversations", groupId, {
      messages: [...mapped, ...live],
      typing: held?.typing ?? {},
    });
    return true;
  } catch (e) {
    console.error("group_dm_messages failed", e);
    return false;
  }
}

/** Once the frame with the group's messages is painted in a focused
 *  window, the Host starts their disappearing timers (plan E7.1), as a
 *  DM's start at `mark_read`; their expiry is then read from the rows. */
export function handleGroupDisplayed(groupId: string): void {
  requestAnimationFrame(() => {
    if (!document.hasFocus()) return;
    commands
      .groupDmMarkRead(groupId)
      .then(() => {
        const messages = groupDmState.conversations[groupId]?.messages ?? [];
        if (awaitsExpiry(messages, disappearingState.timers[groupId]))
          void loadGroupRows(groupId, Math.max(50, messages.length));
      })
      .catch((e) => {
        console.error("group_dm_mark_read failed", e);
      });
  });
}

/** Send `body` to the group; once the Host commits its row, the stored
 *  history (with our queued echo) is read again, and its state then
 *  follows `sendStatus`. `null` on success, else the Host's error. */
export async function handleSendGroupDm(
  groupId: string,
  body: string,
  replyTo: string | null = null
): Promise<string | null> {
  try {
    // Nothing is drawn here: once the send commits, the Host publishes our
    // own row (`messageSent`, plan E6.11) to this window, upserted by its
    // stored id (handlers/group-dm.handlers.ts).
    await afterDraftReports(`group:${groupId}`, () => commands.groupDmSend(groupId, body, replyTo));
    return null;
  } catch (e) {
    return errorMessage(e);
  }
}

/** Report the draft to the group after each edit: whether it holds text.
 *  The Host decides what the members see, as for a peer; typing is best
 *  effort, so a failed report is dropped. */
/** Our group composer changed: whether it holds text, in edit order and
 *  never swallowed (utils/draft_reports.ts). */
export function handleGroupDraftChanged(groupId: string, body: string): void {
  void sendDraftReport(`group:${groupId}`, () =>
    commands.groupDmTyping(groupId, body.trim().length > 0)
  );
}

function failedClientId(groupId: string, messageId: number): string | undefined {
  const m = groupDmState.conversations[groupId]?.messages.find((x) => x.id === messageId);
  return m?.status === "failed" ? m.serverMessageId : undefined;
}

/** Retry our failed group message (`OutboxRetry` takes any outbox row's
 *  client id); its state arrives as `sendStatus`. */
export async function handleRetryGroupSend(groupId: string, messageId: number): Promise<void> {
  const clientId = failedClientId(groupId, messageId);
  if (!clientId) return;
  await commands.outboxRetry(clientId).catch((e) => console.error("Retry failed:", e));
}

/** Discard our failed group message (`OutboxDiscard`, reviewed by the
 *  Host): it stays failed and leaves the group's stream. */
export async function handleDiscardGroupSend(groupId: string, messageId: number): Promise<void> {
  const clientId = failedClientId(groupId, messageId);
  if (!clientId) return;
  await commands.outboxDiscard(clientId).catch((e) => console.error("Discard failed:", e));
}
