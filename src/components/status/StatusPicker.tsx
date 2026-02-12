import { Component, createSignal, For, Show } from "solid-js";
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
  const [open, setOpen] = createSignal(false);

  function handleSelect(status: UserStatus): void {
    commands.setStatus(status);
    setAuthState("status", status);
    setOpen(false);
  }

  function handleToggle(): void {
    setOpen(!open());
  }

  return (
    <div class="status-picker">
      <button class="buddy-item" onClick={handleToggle}>
        <StatusDot status={props.currentStatus} />
        <span class="buddy-name">
          {statusOptions.find((o) => o.value === props.currentStatus)?.label}
        </span>
      </button>
      <Show when={open()}>
        <div class="context-menu status-picker-dropdown">
          <For each={statusOptions}>
            {(option) => (
              <div
                class="context-menu-item"
                onClick={() => handleSelect(option.value)}
              >
                <StatusDot status={option.value} />
                <span class="status-picker-label">{option.label}</span>
              </div>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
};

export default StatusPicker;
