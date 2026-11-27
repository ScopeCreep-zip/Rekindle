import { type Component, For, Show, createEffect, createSignal, on, onCleanup } from "solid-js";
import Modal from "./Modal";
import { commands } from "../../ipc/commands";
import { transferCommands } from "../../ipc/commands/dto_transfer";
import { type LocalHit, type LocalSearchScope, localSearchCommands } from "../../ipc/commands/dto_local_search";
import { localSearchScope, setLocalSearchScope } from "../../stores/local_search.store";
import { communityState } from "../../stores/community.store";
import { handleOpenGroupChat } from "../../actions/group-dm.actions";
import { sendSkinSelection } from "../../actions/skin.actions";
import { addToast } from "../../stores/toast.store";
import { errorMessage } from "../../utils/error";
import { formatTimestamp } from "../../utils/formatting";

/** The panel's title for each scope the Host opens it in. */
function scopeTitle(scope: LocalSearchScope): string {
  switch (scope.kind) {
    case "friends":
      return "Search friends";
    case "servers":
      return "Search servers";
    case "files":
      return "Search files";
    case "conversation":
      return "Search this conversation";
  }
}

/** A hit's words in the list. */
function hitLabel(hit: LocalHit): string {
  switch (hit.kind) {
    case "friend":
    case "groupDm":
    case "member":
    case "file":
      return hit.arg.name;
    case "server":
      return hit.arg.label || hit.arg.address;
    case "message":
      return hit.arg.body;
  }
}

/** What kind of item a hit is, in words (its row's second line). */
function hitKind(hit: LocalHit): string {
  switch (hit.kind) {
    case "friend":
      return "Friend";
    case "groupDm":
      return "Group chat";
    case "member":
      return "Community member";
    case "server":
      return hit.arg.label ? `Server, ${hit.arg.address}` : "Recent server";
    case "file":
      return "File";
    case "message":
      return formatTimestamp(hit.arg.timestamp);
  }
}

/** Show the message `id` in this window when it holds it: scrolled to and highlighted. */
function showHere(id: string | undefined): boolean {
  if (!id) return false;
  const el = document.querySelector(`[data-message-id="${CSS.escape(id)}"]`);
  if (!el) return false;
  el.scrollIntoView({ behavior: "smooth", block: "center" });
  el.classList.add("message-highlight");
  setTimeout(() => el.classList.remove("message-highlight"), 2000);
  return true;
}

/** The community holding channel `channelId`, in this window's community store. */
function communityOfChannel(channelId: string): string | undefined {
  return Object.values(communityState.communities).find((c) => c.channels.some((ch) => ch.id === channelId))?.id;
}

/** Open the item a hit names (plan E7.2 G11): each kind opens where that item lives. */
async function openHit(hit: LocalHit): Promise<void> {
  switch (hit.kind) {
    case "friend":
      return commands.openChatWindow(hit.arg.publicKey);
    case "groupDm":
      return handleOpenGroupChat(hit.arg.groupId);
    case "member":
      return commands.openCommunityWindow(hit.arg.community);
    case "server":
      return sendSkinSelection({ kind: "server", arg: { gameId: hit.arg.gameId, address: hit.arg.address } });
    case "file":
      await transferCommands.start(hit.arg.attachmentId, hit.arg.target);
      return;
    case "message": {
      const m = hit.arg;
      if (showHere(m.messageId)) return;
      if (m.scope === "dm") return commands.openChatWindow(m.conversationId);
      const community = communityOfChannel(m.conversationId);
      // Its community's window; focusing the channel itself takes
      // open_community_window's `channel` (plan E7.2 A, wt/e714gA), passed
      // once that lands in the trunk.
      if (community) return commands.openCommunityWindow(community);
      addToast("That message's community is not open in this window", "info");
      return;
    }
  }
}

/**
 * The skin's Search (plan E7.2 G11): the Host opens it with the scope of the
 * window's view (Friends, Servers, Files) or the conversation; it searches
 * this device's own data as the user types, and a result opens its item.
 */
const LocalSearchPanel: Component = () => {
  const [query, setQuery] = createSignal("");
  const [hits, setHits] = createSignal<LocalHit[]>([]);
  const [error, setError] = createSignal<string | null>(null);
  let generation = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(timer));

  // A new scope starts empty.
  createEffect(
    on(localSearchScope, () => {
      setQuery("");
      setHits([]);
      setError(null);
    }),
  );

  // Search as the user types, after a short pause; only the latest answer shows.
  createEffect(
    on(query, (q) => {
      clearTimeout(timer);
      const scope = localSearchScope();
      if (!scope || !q.trim()) {
        setHits([]);
        return;
      }
      timer = setTimeout(() => {
        const mine = ++generation;
        localSearchCommands.search(scope, q.trim()).then(
          (r) => {
            if (mine !== generation) return;
            setHits(r.hits);
            setError(null);
          },
          (e) => {
            if (mine === generation) setError(errorMessage(e));
          },
        );
      }, 200);
    }),
  );

  const open = (hit: LocalHit): void => {
    setLocalSearchScope(null);
    openHit(hit).catch((e) => addToast(errorMessage(e), "error"));
  };

  return (
    <Show when={localSearchScope()}>
      {(scope) => (
        <Modal isOpen title={scopeTitle(scope())} onClose={() => setLocalSearchScope(null)} size="md">
          <div class="local-search">
            <input
              class="form-input local-search-input"
              type="search"
              placeholder="Type to search"
              aria-label={scopeTitle(scope())}
              value={query()}
              ref={(el) => queueMicrotask(() => el.focus())}
              onInput={(e) => setQuery(e.currentTarget.value)}
            />
            <Show when={error()}>{(message) => <div class="form-error" role="alert">{message()}</div>}</Show>
            <Show when={query().trim() && hits().length === 0 && !error()}>
              <div class="settings-hint">No results</div>
            </Show>
            <ul class="local-search-results" aria-label="Results">
              <For each={hits()}>
                {(hit) => (
                  <li>
                    <button type="button" class="local-search-hit" onClick={() => open(hit)}>
                      <span class="local-search-hit-label">{hitLabel(hit)}</span>
                      <span class="local-search-hit-kind">{hitKind(hit)}</span>
                    </button>
                  </li>
                )}
              </For>
            </ul>
          </div>
        </Modal>
      )}
    </Show>
  );
};

export default LocalSearchPanel;
