import { Component } from "solid-js";
import { buddyListUI, setBuddyListUI } from "../../stores/buddylist-ui.store";

let searchInputRef: HTMLInputElement | undefined;

function handleInput(e: InputEvent): void {
  setBuddyListUI("searchQuery", (e.target as HTMLInputElement).value);
}

function handleKeyDown(e: KeyboardEvent): void {
  if (e.key === "Escape") {
    setBuddyListUI("searchQuery", "");
    searchInputRef?.blur();
  }
}

export function focusSearchInput(): void {
  searchInputRef?.focus();
}

const SearchBar: Component = () => {
  const placeholder = () =>
    buddyListUI.activeTab === "friends" ? "Search friends..." : "Search communities...";

  return (
    <div class="buddy-search-wrapper">
      <input
        ref={searchInputRef}
        class="buddy-search-input"
        type="text"
        placeholder={placeholder()}
        value={buddyListUI.searchQuery}
        onInput={handleInput}
        onKeyDown={handleKeyDown}
      />
    </div>
  );
};

export default SearchBar;
