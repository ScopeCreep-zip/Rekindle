import { type Component, For, Match, Switch } from "solid-js";
import { hasBits } from "../../../ipc/permissions";
import { usePermissionCatalogue } from "../../../stores/permission_catalogue";
import { errorMessage } from "../../../utils/error";

interface PermissionCheckboxListProps {
  permissions: bigint;
  onToggle: (bit: bigint) => void;
}

/** A role's permissions as toggles, grouped and labelled by the Host's
 *  catalogue (contract PN1): the shell declares no permission of its own. */
const PermissionCheckboxList: Component<PermissionCheckboxListProps> = (props) => {
  const catalogue = usePermissionCatalogue();
  return (
    <Switch>
      <Match when={catalogue.error}>
        <div class="settings-hint" role="status">
          Couldn't load the permission list: {errorMessage(catalogue.error)}
        </div>
      </Match>
      <Match when={catalogue.loading}>
        <div class="settings-hint" role="status">
          Loading permissions…
        </div>
      </Match>
      <Match when={catalogue()}>
        {(groups) => (
          <For each={groups()}>
            {(group) => (
              <div>
                <div class="settings-category-label">{group.name}</div>
                <For each={group.permissions}>
                  {(perm) => (
                    <label class="settings-option" data-permission={perm.key}>
                      <input
                        type="checkbox"
                        checked={hasBits(props.permissions, perm.bit)}
                        onChange={() => props.onToggle(perm.bit)}
                      />
                      {perm.label}
                    </label>
                  )}
                </For>
              </div>
            )}
          </For>
        )}
      </Match>
    </Switch>
  );
};

export default PermissionCheckboxList;
