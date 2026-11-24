import { onCleanup, onMount } from "solid-js";
import { dispatchSkinAction, sendSkinSelection } from "../actions/skin.actions";

/** The node a chat window's Mod+F presses: no pack lays a chat-window tree. */
const CHAT_SEARCH_NODE = "ChatSearch";

/**
 * Mod+F in a chat window searches its conversation (plan E7.2 G11). No
 * built-in pack lays a chat-window tree, so nothing there presses the skin's
 * Search; the chord dispatches that same `searchFor` through the Host, which
 * picks the scope from this window (`Views::scope`: a `chat-g…` label is its
 * group; a 1:1 window's is the friend its selection names) and answers
 * `OpenSearch`, which the common runner opens. The scope is never built here.
 * `peer` is the 1:1 window's friend, sent as its selection on mount; a group
 * window passes null.
 */
export function useChatSearchChord(peer: string | null): void {
  const onKeyDown = (e: KeyboardEvent): void => {
    if (!(e.metaKey || e.ctrlKey) || e.shiftKey || e.altKey || e.key.toLowerCase() !== "f") return;
    e.preventDefault();
    void dispatchSkinAction(CHAT_SEARCH_NODE, { kind: "searchFor" });
  };
  onMount(() => {
    if (peer) void sendSkinSelection({ kind: "friend", arg: { publicKey: peer } });
    window.addEventListener("keydown", onKeyDown);
  });
  onCleanup(() => window.removeEventListener("keydown", onKeyDown));
}
