import { type Component, For, Show, onMount } from "solid-js";
import type { Community } from "../../../stores/community.store";
import { commands } from "../../../ipc/commands";
import { handleLoadExpressions } from "../../../actions/community.actions";
import { addToast } from "../../../stores/toast.store";

interface ExpressionsTabProps {
  community: Community;
}

/**
 * The community's emoji, stickers and sounds (plan E7.15), each with
 * Delete: the Host's `ExpressionDelete` checks `MANAGE_EXPRESSIONS` and the
 * intent gate reviews it; the list is the Host's expression list, re-read
 * after a delete.
 */
const ExpressionsTab: Component<ExpressionsTabProps> = (props) => {
  onMount(() => void handleLoadExpressions(props.community.id));

  async function remove(id: string, name: string): Promise<void> {
    try {
      await commands.deleteEmoji(props.community.id, id);
      await handleLoadExpressions(props.community.id);
      addToast(`:${name}: deleted`, "success");
    } catch (e) {
      console.error("Failed to delete expression:", e);
      addToast(typeof e === "string" ? e : "Failed to delete", "error");
    }
  }

  return (
    <div class="settings-section">
      <h3 class="settings-subheading">Emoji, stickers and sounds</h3>
      <Show
        when={props.community.expressions.length > 0}
        fallback={<div class="settings-hint">This community has none yet.</div>}
      >
        <For each={props.community.expressions}>
          {(x) => (
            <div class="settings-list-item">
              <span>
                :{x.name}: ({x.kind})
              </span>
              <button
                type="button"
                class="form-btn-danger"
                aria-label={`Delete ${x.kind} ${x.name}`}
                onClick={() => void remove(x.id, x.name)}
              >
                Delete
              </button>
            </div>
          )}
        </For>
      </Show>
    </div>
  );
};

export default ExpressionsTab;
