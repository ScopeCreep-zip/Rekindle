import { Component, For, Show, createSignal, onCleanup, onMount } from "solid-js";

export interface ContextMenuItem {
  label: string;
  action: () => void;
  danger?: boolean;
}

interface ContextMenuProps {
  items: ContextMenuItem[];
  x: number;
  y: number;
  onClose: () => void;
}

const ContextMenu: Component<ContextMenuProps> = (props) => {
  let menuRef: HTMLDivElement | undefined;

  function handleClickOutside(e: MouseEvent): void {
    if (menuRef && !menuRef.contains(e.target as Node)) {
      props.onClose();
    }
  }

  onMount(() => {
    document.addEventListener("mousedown", handleClickOutside);
  });

  onCleanup(() => {
    document.removeEventListener("mousedown", handleClickOutside);
  });

  return (
    <div
      class="context-menu"
      ref={menuRef}
      style={{
        left: `${props.x}px`,
        top: `${props.y}px`,
      }}
    >
      <For each={props.items}>
        {(item) => (
          <div
            class={item.danger ? "context-menu-item context-menu-item-danger" : "context-menu-item"}
            onClick={() => {
              item.action();
              props.onClose();
            }}
          >
            {item.label}
          </div>
        )}
      </For>
    </div>
  );
};

export default ContextMenu;
