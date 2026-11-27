import {
  type Component,
  type JSX,
  Show,
  children,
  createContext,
  createSignal,
  useContext,
} from "solid-js";
import { Portal } from "solid-js/web";
import type { SlotKey } from "../../ipc/commands/dto_skin_tree";

/**
 * The places a skin tree lays for the shell's own content (plan SK17-2, the
 * chat window as a skin tree): the pack's ChatWindow tiles `TypingMessage`,
 * `ChatArea`, `ChatEdit`, its send button and its InfoView button. The Host
 * serves a node for each it lays (`NodeRole` `slot`); the shell mounts its
 * part there.
 */
export type { SlotKey };

interface SlotTargets {
  target: (key: SlotKey) => HTMLElement | undefined;
  register: (key: SlotKey, el: HTMLElement | undefined) => void;
}

const SlotContext = createContext<SlotTargets>();

/**
 * The slot elements of the tree inside it: the tree's slot nodes register
 * themselves (`useSlotRegister`), and every `SlotMount` below reads them.
 */
export const SlotProvider: Component<{ children: JSX.Element }> = (props) => {
  const [targets, setTargets] = createSignal<Partial<Record<SlotKey, HTMLElement>>>({});
  const value: SlotTargets = {
    target: (key) => targets()[key],
    register: (key, el) => setTargets((t) => ({ ...t, [key]: el })),
  };
  return <SlotContext.Provider value={value}>{props.children}</SlotContext.Provider>;
};

/** A slot node's ref: registers its element as the slot `key`'s target. */
export function useSlotRegister():
  | ((key: SlotKey, el: HTMLElement | undefined) => void)
  | undefined {
  return useContext(SlotContext)?.register;
}

/**
 * The shell's part for slot `key`: inside the tree's slot node when the
 * window's tree lays one, else where it stands (a window without a tree, or
 * a pack that lays no such slot). A portal keeps the part's own reactive
 * owner, so its state and handlers are the chat's wherever it is drawn.
 *
 * The part is created once (`children` resolves it): the fallback and the
 * portal draw the same nodes, so a part drawn in place before the tree's
 * slot registers keeps its state (a draft typed in the composer) when it
 * moves into the slot. The portal's container (`skin-slot-mount`) takes the
 * slot's whole box as a flex column, so a part that grows (`flex: 1`)
 * fills the slot, and one window mounts each slot once.
 */
export const SlotMount: Component<{ slot: SlotKey; children: JSX.Element }> = (props) => {
  const slots = useContext(SlotContext);
  const target = (): HTMLElement | undefined => slots?.target(props.slot);
  const part = children(() => props.children);
  return (
    <Show when={target()} fallback={part()}>
      {(el) => (
        <Portal
          mount={el()}
          ref={(container: HTMLDivElement) => container.classList.add("skin-slot-mount")}
        >
          {part()}
        </Portal>
      )}
    </Show>
  );
};
