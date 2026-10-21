import { reconcile } from "solid-js/store";
import { commands, type GroupDmView } from "../ipc/commands";
import { groupDmState, setGroupDmState } from "../stores/group-dm.store";
import { transformDmMessage } from "../utils/transformers";
import { errorMessage } from "../utils/error";

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
export const handleLeaveGroupDm = (groupId: string) =>
  answer(() => commands.groupDmLeave(groupId));

export function handleOpenGroupChat(groupId: string): void {
  void commands.openGroupChatWindow(groupId);
}

/** Load the group's newest `limit` stored messages. */
export async function handleLoadGroupMessages(groupId: string, limit: number): Promise<void> {
  try {
    const rows = await commands.groupDmMessages(groupId, limit);
    const typing = groupDmState.conversations[groupId]?.typing ?? {};
    setGroupDmState("conversations", groupId, {
      messages: rows.map(transformDmMessage),
      typing,
    });
  } catch (e) {
    console.error("group_dm_messages failed", e);
  }
}
