import { Component, For, JSX, Show, createEffect, createSignal } from "solid-js";
import { DropdownMenu } from "@kobalte/core/dropdown-menu";
import { authState } from "../../stores/auth.store";
import { handleSetStatusMessage } from "../../actions/auth.actions";
import { isNetworkConnected, networkLabel, networkState, routePausedText, trackNetworkStatus } from "../../stores/network.store";
import StatusDot from "../status/StatusDot";
import { handleSelectStatus, statusOptions } from "../status/StatusPicker";

/**
 * N3 (design §3), the pack's `StatusEdit`: type an away message and press
 * Enter to set it. While the node is detached from the network the field
 * says so ("Connecting...", Xfire showed connecting in the status field),
 * which replaces the network banner. While the Host says our inbound route
 * is paused (owner decision 65, plan RV1) the field says that instead, in
 * the app layout's status line words and ground, with no dismiss, until the
 * Host lifts it: the pack's status field is the tree's status area. The
 * field is the tree's only way to set an away message, so the input stays
 * (decision 49): a press passes through the line to it, and the line steps
 * aside while the input has focus.
 */
export const StatusText: Component<{ attrs: Record<string, unknown> }> = (props) => {
  trackNetworkStatus();
  const [value, setValue] = createSignal("");
  // The input has focus: the paused line steps aside while the away message
  // is typed (focus events, not :focus-within, which WebKit leaves unmatched
  // while the window lacks system focus).
  const [editing, setEditing] = createSignal(false);
  createEffect(() => setValue(authState.statusMessage ?? ""));
  const shown = (): string =>
    isNetworkConnected()
      ? (statusOptions.find((o) => o.value === authState.status)?.label ?? "")
      : networkLabel();
  return (
    <div {...props.attrs} class="skin-status">
      <Show when={networkState.routePaused && !editing()}>
        <div class="skin-status-paused" role="alert">
          <span class="skin-status-paused-text">{routePausedText(networkState.routePauseCause)}</span>
        </div>
      </Show>
      <input
        class="skin-status-edit"
        title={networkState.routePaused ? routePausedText(networkState.routePauseCause) : undefined}
        aria-label="Status message"
        placeholder={shown()}
        value={value()}
        onInput={(e) => setValue(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") void handleSetStatusMessage(value());
          else if (e.key === "Escape") setValue(authState.statusMessage ?? "");
        }}
        onFocus={() => setEditing(true)}
        onBlur={() => {
          setEditing(false);
          setValue(authState.statusMessage ?? "");
        }}
      />
    </div>
  );
};

/** N3's `DownArrow`: opens Online/Away/Busy/Appear Offline. */
export const StatusList: Component<{ attrs: Record<string, unknown>; children: JSX.Element }> = (props) => (
  <DropdownMenu placement="bottom-end">
    <DropdownMenu.Trigger class="skin-status-list" {...props.attrs} aria-label="Status">
      {props.children}
    </DropdownMenu.Trigger>
    <DropdownMenu.Portal>
      <DropdownMenu.Content class="context-menu status-picker-dropdown">
        <DropdownMenu.RadioGroup value={authState.status} onChange={handleSelectStatus}>
          <For each={statusOptions}>
            {(option) => (
              <DropdownMenu.RadioItem class="context-menu-item" value={option.value} closeOnSelect>
                <StatusDot status={option.value} />
                <span class="status-picker-label">{option.label}</span>
              </DropdownMenu.RadioItem>
            )}
          </For>
        </DropdownMenu.RadioGroup>
      </DropdownMenu.Content>
    </DropdownMenu.Portal>
  </DropdownMenu>
);
