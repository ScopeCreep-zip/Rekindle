import { Component, For } from "solid-js";
import { PERMISSION_CATEGORIES, hasPermission } from "../../../ipc/permissions";

interface PermissionCheckboxListProps {
  permissions: bigint;
  onToggle: (bit: bigint) => void;
}

const PermissionCheckboxList: Component<PermissionCheckboxListProps> = (props) => (
  <For each={PERMISSION_CATEGORIES}>
    {(category) => (
      <div>
        <div class="settings-category-label">
          {category.name}
        </div>
        <For each={category.permissions}>
          {(perm) => (
            <label class="settings-option">
              <input
                type="checkbox"
                checked={hasPermission(props.permissions, perm.value)}
                onChange={() => props.onToggle(perm.value)}
              />
              {perm.label}
            </label>
          )}
        </For>
      </div>
    )}
  </For>
);

export default PermissionCheckboxList;
