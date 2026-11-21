import { Component, For } from "solid-js";
import { DropdownMenu } from "@kobalte/core/dropdown-menu";
import { commands } from "../../ipc/commands";
import { setAuthState } from "../../stores/auth.store";
import { addToast } from "../../stores/toast.store";
import { errorMessage } from "../../utils/error";
import type { UserStatus } from "../../stores/auth.store";
import StatusDot from "./StatusDot";

interface StatusOption {
  value: UserStatus;
  label: string;
}

export const statusOptions: StatusOption[] = [
  { value: "online", label: "Online" },
  { value: "away", label: "Away" },
  { value: "busy", label: "Busy" },
  // Invisible: peers see offline; only logging out sets offline.
  { value: "invisible", label: "Appear Offline" },
];

interface StatusPickerProps {
  currentStatus: UserStatus;
}

/** Choose our status. The Host owns it: show what it answers, and its
 *  refusal (the label keeps the status it still holds). */
export function handleSelectStatus(value: string): void {
  void commands
    .setStatus(value as Exclude<UserStatus, "offline">)
    .then((view) => setAuthState("status", view.status))
    .catch((e: unknown) => addToast(`Status not changed: ${errorMessage(e)}`, "error"));
}

const StatusPicker: Component<StatusPickerProps> = (props) => {

  return (
    <DropdownMenu placement="top-start">
      <DropdownMenu.Trigger class="buddy-item status-picker">
        <StatusDot status={props.currentStatus} />
        <span class="buddy-name">
          {statusOptions.find((o) => o.value === props.currentStatus)?.label}
        </span>
        {/* Plan E7.7.7: a skin's status arrow (slot `statusbar.arrow`); 0 × 0 without one. */}
        <span class="status-picker-arrow" aria-hidden="true" />
      </DropdownMenu.Trigger>
      <DropdownMenu.Portal>
        <DropdownMenu.Content class="context-menu status-picker-dropdown">
          <DropdownMenu.RadioGroup value={props.currentStatus} onChange={handleSelectStatus}>
            <For each={statusOptions}>
              {(option) => (
                // A radio item keeps its menu open by default (Kobalte
                // `MenuRadioItem`, closeOnSelect false): choosing a status
                // closes the picker, as Xfire's did.
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
};

export default StatusPicker;
