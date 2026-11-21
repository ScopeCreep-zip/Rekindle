import { Component, JSX, Show, createSignal } from "solid-js";
import { authState } from "../../stores/auth.store";
import { handleSetDisplayName } from "../../actions/auth.actions";

/**
 * N1 (design §3): our display name in the pack's `UserNameField`. It
 * shows the name; a double-click edits it (Xfire's name field was
 * `Default="Activate"`), Enter saves and Escape cancels.
 */
const NameField: Component<{ id: string; children: JSX.Element }> = (props) => {
  const [editing, setEditing] = createSignal(false);
  const [value, setValue] = createSignal("");
  function start(): void {
    setValue(authState.displayName ?? "");
    setEditing(true);
  }
  async function commit(): Promise<void> {
    await handleSetDisplayName(value());
    setEditing(false);
  }
  return (
    <div data-skin-node={props.id} class="skin-name" onDblClick={start}>
      {props.children}
      <Show
        when={editing()}
        fallback={
          <span class="skin-name-text" role="heading" aria-level="1" title="Double-click to rename">
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
