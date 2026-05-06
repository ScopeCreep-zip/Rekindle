import { Component, For } from "solid-js";
import { DropdownMenu } from "@kobalte/core/dropdown-menu";
import { commands } from "../../ipc/commands";
import { setAuthState } from "../../stores/auth.store";
import type { UserStatus } from "../../stores/auth.store";
import StatusDot from "./StatusDot";

interface StatusOption {
  value: UserStatus;
  label: string;
}

const statusOptions: StatusOption[] = [
  { value: "online", label: "Online" },
  { value: "away", label: "Away" },
  { value: "busy", label: "Busy" },
  { value: "offline", label: "Appear Offline" },
];

interface StatusPickerProps {
  currentStatus: UserStatus;
}

const StatusPicker: Component<StatusPickerProps> = (props) => {
  function handleSelect(value: string): void {
    const status = value as UserStatus;
    commands.setStatus(status);
    setAuthState("status", status);
  }

  return (
    <DropdownMenu placement="top-start">
      <DropdownMenu.Trigger class="buddy-item status-picker">
        <StatusDot status={props.currentStatus} />
        <span class="buddy-name">
          {statusOptions.find((o) => o.value === props.currentStatus)?.label}
        </span>
      </DropdownMenu.Trigger>
      <DropdownMenu.Portal>
        <DropdownMenu.Content class="context-menu status-picker-dropdown">
          <DropdownMenu.RadioGroup value={props.currentStatus} onChange={handleSelect}>
            <For each={statusOptions}>
              {(option) => (
                <DropdownMenu.RadioItem class="context-menu-item" value={option.value}>
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
};

export default StatusPicker;
