import { createStore } from "solid-js/store";
import type { GroupDmView } from "../ipc/commands";
import type { Message } from "./chat.store";

/** One group's chat window state: its stored history and who is typing
 *  (member key → typing), as the backend reports them. */
export interface GroupConversation {
  messages: Message[];
  typing: Record<string, boolean>;
}

/** Group DMs (plan E2.6): the groups and pending invites the Host lists,
 *  keyed by group id, and each open group's conversation. Nothing here is
 *  decided by the window; it mirrors `group_dm_list`, `group_dm_messages`
 *  and the `GroupDmEvent`s. */
export interface GroupDmState {
  groups: Record<string, GroupDmView>;
  conversations: Record<string, GroupConversation>;
}

export const [groupDmState, setGroupDmState] = createStore<GroupDmState>({
  groups: {},
  conversations: {},
});
