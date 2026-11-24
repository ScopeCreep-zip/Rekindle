import { createStore } from "solid-js/store";

/** The app's own layout's tabs: Xfire's Friends, Servers and Downloads, and Communities. */
export type BuddyListTab = "friends" | "communities" | "servers" | "files";

export interface BuddyListUIState {
  activeTab: BuddyListTab;
  searchQuery: string;
  showCreateCommunity: boolean;
  showJoinCommunity: boolean;
  /** Wave 12 W12.10 — multi-friend group-call picker modal. */
  showStartGroupCall: boolean;
  /** The "New group" dialog (plan E2.6). */
  showNewGroup: boolean;
  /** The friend a skin's New Room named (its selection), picked when the dialog opens. */
  newGroupPeer: string | null;
  /** The group a skin's Invite Users to Room opened the picker for (G8). */
  inviteToRoom: string | null;
  /** The notification center's panel (a skin's OpenNotifications too). */
  showNotifications: boolean;
  /** The Communities view's New chooser (plan CB1): create, or join with an invite. */
  showCommunityStart: boolean;
}

const [buddyListUI, setBuddyListUI] = createStore<BuddyListUIState>({
  activeTab: "friends",
  searchQuery: "",
  showCreateCommunity: false,
  showJoinCommunity: false,
  showStartGroupCall: false,
  showNewGroup: false,
  newGroupPeer: null,
  inviteToRoom: null,
  showNotifications: false,
  showCommunityStart: false,
});

export function switchTab(tab: BuddyListTab): void {
  setBuddyListUI("activeTab", tab);
  setBuddyListUI("searchQuery", "");
}

export { buddyListUI, setBuddyListUI };
