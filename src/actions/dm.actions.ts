// DM user actions — list, start, accept, decline, send.
//
// Split from `handlers/dm.handlers.ts`, which also held
// `subscribeDmInbox`. That mixed two tiers in one module: a component
// wiring a "Send" button had to import the same file that registers the
// chat-event subscription at app start.

import { commands } from "../ipc/commands";
import { dmState, setDmState } from "../stores/dm.store";
import { notificationState } from "../stores/notification.store";

export async function handleListDms(): Promise<void> {
  const list = await commands.listDms();
  const map: Record<string, (typeof list)[number]> = {};
  for (const conv of list) {
    map[conv.recordKey] = conv;
  }
  setDmState("conversations", map);
}

export async function handleStartDm(
  bobPublicKey: string,
  alicePseudonym: string,
): Promise<string> {
  const recordKey = await commands.startDm(bobPublicKey, alicePseudonym);
  await handleListDms();
  setDmState("activeRecordKey", recordKey);
  await commands.openDmWindow(recordKey, alicePseudonym);
  return recordKey;
}

export async function handleAcceptDm(recordKey: string): Promise<void> {
  await commands.acceptDmInvite(recordKey);
  setDmState("pendingInvites", recordKey, undefined!);
  await handleListDms();
  setDmState("activeRecordKey", recordKey);
  const conv = dmState.conversations[recordKey];
  const title = conv?.initiatorPseudonym ?? recordKey.slice(0, 12);
  await commands.openDmWindow(recordKey, title);
}

export async function handleDeclineDm(recordKey: string): Promise<void> {
  await commands.declineDmInvite(recordKey);
  setDmState("pendingInvites", recordKey, undefined!);
}

export async function handleSendDm(recordKey: string, body: string): Promise<void> {
  await commands.sendDmMessage(recordKey, body);
  // The backend re-emits MessageReceived for the local write so the
  // subscribeDmInbox handler will append the message — no need to mutate
  // the store here; that keeps a single source of truth for ordering.
}

// Re-export for components that surface the current notification badge.
export { notificationState };
