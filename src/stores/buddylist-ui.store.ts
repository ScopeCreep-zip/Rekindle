import { createStore } from "solid-js/store";

export type BuddyListTab = "friends" | "communities";

export interface BuddyListUIState {
  activeTab: BuddyListTab;
  searchQuery: string;
  menuOpen: string | null;
  showCreateCommunity: boolean;
  showJoinCommunity: boolean;
}

const [buddyListUI, setBuddyListUI] = createStore<BuddyListUIState>({
  activeTab: "friends",
  searchQuery: "",
  menuOpen: null,
  showCreateCommunity: false,
  showJoinCommunity: false,
});

export function switchTab(tab: BuddyListTab): void {
  setBuddyListUI("activeTab", tab);
  setBuddyListUI("searchQuery", "");
}

export { buddyListUI, setBuddyListUI };
