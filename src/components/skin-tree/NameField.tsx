import { Component, JSX, Show, createEffect, createSignal } from "solid-js";
import { authState } from "../../stores/auth.store";
import { handleSetDisplayName } from "../../actions/auth.actions";

/**
 * N1 (design §3): our display name in the pack's `UserNameField`. It
 * shows the name; the pack's own trigger edits it (`EditName`, the default
 * pack's double-click; plan SK2-A5), Enter saves and Escape cancels.
 * `edit` counts the edit requests: each new one opens the editor.
 */
const NameField: Component<{ attrs: Record<string, unknown>; edit: number; children: JSX.Element }> = (props) => {
  const [editing, setEditing] = createSignal(false);
  const [value, setValue] = createSignal("");
  function start(): void {
    setValue(authState.displayName ?? "");
    setEditing(true);
  }
  createEffect((seen: number) => {
    if (props.edit !== seen) start();
    return props.edit;
  }, props.edit);
  async function commit(): Promise<void> {
    await handleSetDisplayName(value());
    setEditing(false);
  }
  return (
    <div {...props.attrs} class="skin-name">
      {props.children}
      <Show
        when={editing()}
        fallback={
          <span class="skin-name-text" role="heading" aria-level="1">
            {authState.displayName ?? "Unknown"}
          </span>
        }
      >
        <input
          class="skin-name-edit"
          aria-label="Display name"
          value={value()}
          ref={(el) => queueMicrotask(() => el.select())}
          onInput={(e) => setValue(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void commit();
            else if (e.key === "Escape") setEditing(false);
          }}
          onBlur={() => void commit()}
        />
      </Show>
    </div>
  );
};

export default NameField;
