import { Component, For, JSX, createEffect, createSignal } from "solid-js";
import { DropdownMenu } from "@kobalte/core/dropdown-menu";
import { authState } from "../../stores/auth.store";
import { handleSetStatusMessage } from "../../actions/auth.actions";
import { isNetworkConnected, networkLabel, trackNetworkStatus } from "../../stores/network.store";
import StatusDot from "../status/StatusDot";
import { handleSelectStatus, statusOptions } from "../status/StatusPicker";

/**
 * N3 (design §3), the pack's `StatusEdit`: type an away message and press
 * Enter to set it. While the node is detached from the network the field
 * says so ("Connecting...", Xfire showed connecting in the status field),
 * which replaces the network banner.
 */
export const StatusText: Component<{ attrs: Record<string, unknown> }> = (props) => {
  trackNetworkStatus();
  const [value, setValue] = createSignal("");
  createEffect(() => setValue(authState.statusMessage ?? ""));
  const shown = (): string =>
    isNetworkConnected()
      ? (statusOptions.find((o) => o.value === authState.status)?.label ?? "")
      : networkLabel();
  return (
    <div {...props.attrs} class="skin-status">
      <input
        class="skin-status-edit"
        aria-label="Status message"
        placeholder={shown()}
        value={value()}
        onInput={(e) => setValue(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") void handleSetStatusMessage(value());
          else if (e.key === "Escape") setValue(authState.statusMessage ?? "");
        }}
        onBlur={() => setValue(authState.statusMessage ?? "")}
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
