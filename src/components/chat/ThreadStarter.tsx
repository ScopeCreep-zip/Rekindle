import { type Component, Show } from "solid-js";
import SkinGlyph from "../common/SkinGlyph";

interface ThreadStarterProps {
  threadName: string;
  messageCount: number;
  /** Archived threads keep their badge (Discord): the starter message
   *  still opens the thread, which shows Unarchive (plan E7.13 walk 22). */
  archived: boolean;
  onClick: () => void;
}

const ThreadStarter: Component<ThreadStarterProps> = (props) => {
  return (
    <button
      type="button"
      class="thread-starter-badge"
      classList={{ "thread-starter-archived": props.archived }}
      onClick={props.onClick}
    >
      <SkinGlyph name="thread" />
      {props.threadName} ({props.messageCount})
      <Show when={props.archived}>{" · archived"}</Show>
    </button>
  );
};

export default ThreadStarter;
