import { type Component, For } from "solid-js";
import { buddyListUI, switchTab, type BuddyListTab } from "../../stores/buddylist-ui.store";

/** The app's own layout's tabs (a skin's tree switches its views itself):
 *  Xfire's Friends, Servers and Downloads, and Rekindle's Communities. */
const TABS: { tab: BuddyListTab; label: string }[] = [
  { tab: "friends", label: "Friends" },
  { tab: "communities", label: "Communities" },
  { tab: "servers", label: "Servers" },
  { tab: "files", label: "Downloads" },
];

const TabBar: Component = () => (
  <div class="buddy-tab-bar" role="tablist" aria-label="Buddy list">
    <For each={TABS}>
      {(t, i) => (
        <button
          class={`buddy-tab ${buddyListUI.activeTab === t.tab ? "buddy-tab-active" : ""}`}
          role="tab"
          aria-selected={buddyListUI.activeTab === t.tab}
          onClick={() => switchTab(t.tab)}
          title={`${t.label} (Alt+${i() + 1})`}
        >
          {t.label}
        </button>
      )}
    </For>
  </div>
);

export default TabBar;
