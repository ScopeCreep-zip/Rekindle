import { createStore } from "solid-js/store";

export type BuddyListTab = "friends" | "communities";

export interface BuddyListUIState {
  activeTab: BuddyListTab;
  searchQuery: string;
  showCreateCommunity: boolean;
  showJoinCommunity: boolean;
}

const [buddyListUI, setBuddyListUI] = createStore<BuddyListUIState>({
  activeTab: "friends",
  searchQuery: "",
  showCreateCommunity: false,
  showJoinCommunity: false,
});

export function switchTab(tab: BuddyListTab): void {
  setBuddyListUI("activeTab", tab);
  setBuddyListUI("searchQuery", "");
}

export { buddyListUI, setBuddyListUI };
