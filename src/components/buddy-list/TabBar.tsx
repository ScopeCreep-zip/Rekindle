import { Component } from "solid-js";
import { buddyListUI, switchTab } from "../../stores/buddylist-ui.store";

const TabBar: Component = () => {
  function handleFriendsTab(): void {
    switchTab("friends");
  }

  function handleCommunitiesTab(): void {
    switchTab("communities");
  }

  return (
    <div class="buddy-tab-bar">
      <button
        class={`buddy-tab ${buddyListUI.activeTab === "friends" ? "buddy-tab-active" : ""}`}
        onClick={handleFriendsTab}
        title="Friends (Alt+1)"
      >
        Friends
      </button>
      <button
        class={`buddy-tab ${buddyListUI.activeTab === "communities" ? "buddy-tab-active" : ""}`}
        onClick={handleCommunitiesTab}
        title="Communities (Alt+2)"
      >
        Communities
      </button>
    </div>
  );
};

export default TabBar;
