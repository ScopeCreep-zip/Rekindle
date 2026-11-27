// Group DMs (plan E2.6): the Host owns the roster, keys, validation and
// every policy; these are thin wrappers. Create, accept, decline and leave
// are reviewed by the Host (`hostInvoke` presents the intent dialog).
import { hostInvoke } from "../invoke";
import type { DisappearingTimerView, DmMessage } from "./dto_dm";

/** One member of a group, labelled by the Host (plan E2.6 GC1): "You", a
 *  friend's name, or the roster's name marked "(not a friend)"; never a key. */
interface GroupDmMemberView {
  key: string;
  label: string;
}

/** A group we are in, or an invite we have not answered (`pending`). */
export interface GroupDmView {
  groupId: string;
  name: string;
  members: GroupDmMemberView[];
  epoch: number;
  pending: boolean;
  /** The friend whose chat the group was created from (its attached room). */
  originPeer: string | null;
  /** In our "Chat Rooms" favourites (local only). */
  favourite: boolean;
  /** The Host's verdict on renaming it: null when we may, else why not. */
  renameRefused: string | null;
  /** The Host's verdict on inviting more to it: null when we may, else why not. */
  inviteRefused: string | null;
  /** The Host's verdict on starting a call in it: null when we may, else why not. */
  callRefused: string | null;
}

export const groupDmCommands = {
  /** The new group's id (32 hex). */
  /** `originPeer`: the member whose chat it comes from (its attached room). */
  groupDmCreate: (name: string, members: string[], originPeer?: string) =>
    hostInvoke<string>("group_dm_create", { name, members, originPeer: originPeer ?? null }),
  groupDmAccept: (groupId: string) => hostInvoke<unknown>("group_dm_accept", { groupId }),
  groupDmDecline: (groupId: string) => hostInvoke<unknown>("group_dm_decline", { groupId }),
  groupDmLeave: (groupId: string) => hostInvoke<unknown>("group_dm_leave", { groupId }),
  groupDmRename: (groupId: string, name: string) =>
    hostInvoke<unknown>("group_dm_rename", { groupId, name }),
  /** Invite friends into a group we created (at most eight in all). */
  groupDmInvite: (groupId: string, members: string[]) =>
    hostInvoke<unknown>("group_dm_invite", { groupId, members }),
  /** Our message's client id once its row and echo are committed
   *  (`queued`, then `sendStatus` events). */
  groupDmSend: (groupId: string, body: string, replyTo: string | null = null) =>
    hostInvoke<string>("group_dm_send", { groupId, body, replyTo }),
  /** Whether our draft to the group holds text; the Host decides what goes
   *  out. */
  groupDmTyping: (groupId: string, typing: boolean) =>
    hostInvoke<unknown>("group_dm_typing", { groupId, typing }),
  groupDmList: () => hostInvoke<GroupDmView[]>("group_dm_list"),
  /** The newest `limit` stored messages of the group before `before` (ms),
   *  oldest first. */
  groupDmMessages: (groupId: string, limit: number, before?: number) =>
    hostInvoke<DmMessage[]>("group_dm_messages", { groupId, limit, before: before ?? null }),
  /** The group window painted its messages: received disappearing timers
   *  start (plan E7.1); a history load alone starts none. */
  groupDmMarkRead: (groupId: string) => hostInvoke<unknown>("group_dm_mark_read", { groupId }),
  /** The disappearing timer of a DM (the friend's key) or group DM (its
   *  id), with the presets the control offers (plan E7.1). */
  disappearingGet: (conversation: string) =>
    hostInvoke<DisappearingTimerView>("disappearing_get", { conversation }),
  /** Set that timer (0 = off); reviewed by the Host (`SetDisappearingTimer`),
   *  then every window hears `disappearingTimerChanged`. */
  disappearingSet: (conversation: string, seconds: number) =>
    hostInvoke<unknown>("disappearing_set", { conversation, seconds }),
};
