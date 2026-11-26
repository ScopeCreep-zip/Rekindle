import { Component, For, Match, Show, Switch, createSignal, onCleanup, onMount } from "solid-js";
import { commands } from "../../ipc/commands";
import { subscribeInfoview } from "../../ipc/channels";
import type { InfoviewAction, InfoviewRequest, InfoviewSection, InfoviewView } from "../../ipc/commands/dto_infoview";
import type { FriendInfo } from "../../ipc/commands/types";
import { addToast } from "../../stores/toast.store";
import { formatDuration } from "../../utils/formatting";
import { handleLaunchGame } from "../../actions/game.actions";
import { notJoinableText } from "../../utils/joinable_text";
import ActivityFeed from "./ActivityFeed";

/**
 * The Infoview (plan E7.15 IV6, design §2): the sections the Host composes
 * for this window's subject, in the Host's order, read once
 * (`get_infoview`) and then replaced by each `infoview-event` the Host
 * pushes to this window. Each link is the Host's typed action, sent as it
 * is; the window decides nothing about what a section shows or allows.
 */
const Infoview: Component<{ subject: InfoviewRequest }> = (props) => {
  const [view, setView] = createSignal<InfoviewView | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  let unlisten: (() => void) | undefined;
  onMount(() => {
    void subscribeInfoview(setView).then((u) => (unlisten = u));
    commands
      .getInfoview(props.subject)
      .then(setView)
      .catch((e) => setError(`Infoview not read: ${String(e)}`));
  });
  onCleanup(() => unlisten?.());
  return (
    <div class="infoview">
      <Show when={error()}>
        <div class="form-error" role="alert">
          {error()}
        </div>
      </Show>
      <For each={view()?.sections ?? []}>{(section) => <Section section={section} />}</For>
    </div>
  );
};

/** Send one of the Host's actions. `activityMore` pages inside the
 *  Activity section itself; `joinServer` launches through the Host's review
 *  with the source the Host stated for the row. */
function run(action: InfoviewAction): void {
  const failed = (e: unknown) => addToast(String(e), "error");
  switch (action.type) {
    case "openCommunity":
      commands.openCommunityWindow(action.community).catch(failed);
      return;
    case "openChat":
      commands.openChatWindow(action.publicKey).catch(failed);
      return;
    case "openProfile":
      commands.openProfileWindow(action.publicKey).catch(failed);
      return;
    case "joinServer":
      if (action.gameId !== null) void handleLaunchGame(action.gameId, action.server, action.source);
      return;
    case "activityMore":
      return;
  }
}

/** An action's link text. */
function actionText(action: InfoviewAction): string {
  switch (action.type) {
    case "openCommunity":
      return "Open";
    case "openChat":
      return "Send Message";
    case "openProfile":
      return "View Profile";
    case "joinServer":
      return "Join Game";
    case "activityMore":
      return "View More";
  }
}

/**
 * A row's Join (plan E7.15 IV3c/JV1): always shown; the Host's verdict on
 * the action itself enables it or disables it (aria-disabled, focusable)
 * with its reason beside it. Pressed, it launches through the Host's review.
 */
const JoinLink: Component<{ action: InfoviewAction }> = (props) => {
  const reason = (): string => {
    const a = props.action;
    return a.type === "joinServer" && !a.joinable ? notJoinableText(a) : "";
  };
  return (
    <>
      <button
        type="button"
        class="infoview-link"
        aria-disabled={reason() ? "true" : undefined}
        title={reason() || undefined}
        onClick={() => !reason() && run(props.action)}
      >
        {actionText(props.action)}
      </button>
      <Show when={reason()}>
        <span class="infoview-meta">{reason()}</span>
      </Show>
    </>
  );
};

function friendName(f: FriendInfo): string {
  return f.nickname ?? f.displayName;
}

const Section: Component<{ section: InfoviewSection }> = (props) => (
  <Switch>
    <Match when={props.section.type === "activity" && props.section}>
      {(s) => <ActivityFeed entries={s().entries} more={s().more} />}
    </Match>
    <Match when={props.section.type === "communities" && props.section}>
      {(s) => (
        <section class="infoview-section" aria-label="Your communities">
          <div class="infoview-section-title">Your Communities</div>
          <Show when={s().rows.length > 0} fallback={<div class="infoview-empty">No communities yet.</div>}>
            <ul class="infoview-list">
              <For each={s().rows}>
                {(row) => (
                  <li class="infoview-row">
                    <button type="button" class="infoview-link" onClick={() => run(row.action)}>
                      {row.name}
                    </button>
                    <Show when={row.unread > 0}>
                      <span class="infoview-meta">
                        {row.unread} unread{row.mentions > 0 ? `, ${row.mentions} mentions` : ""}
                      </span>
                    </Show>
                    <Show when={row.nextEvent}>
                      {(ev) => <span class="infoview-meta">Next: {ev().title}</span>}
                    </Show>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </section>
      )}
    </Match>
    <Match when={props.section.type === "nowPlaying" && props.section}>
      {(s) => (
        <section class="infoview-section" aria-label="Friends playing now">
          <div class="infoview-section-title">Friends Playing Now</div>
          <Show when={s().rows.length > 0} fallback={<div class="infoview-empty">No friends in a game.</div>}>
            <ul class="infoview-list">
              <For each={s().rows}>
                {(row) => (
                  <li class="infoview-row">
                    <span class="infoview-text">
                      {friendName(row.friend)}: {row.gameName}
                      {row.serverInfo ? ` (${row.serverInfo})` : ""}
                    </span>
                    <span class="infoview-meta">{formatDuration(row.elapsedSeconds)}</span>
                    <JoinLink action={row.join} />
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </section>
      )}
    </Match>
    <Match when={props.section.type === "friendHeader" && props.section}>
      {(s) => (
        <section class="infoview-section" aria-label={friendName(s().friend)}>
          <div class="infoview-section-title">{friendName(s().friend)}</div>
          <Show when={s().friend.nickname}>
            <div class="infoview-meta">{s().friend.displayName}</div>
          </Show>
          <div class="infoview-meta">{s().blocked ? "Blocked" : s().friend.statusMessage ?? s().friend.status}</div>
          <div class="infoview-actions">
            <For each={s().actions}>
              {(a) => (
                <button type="button" class="infoview-link" onClick={() => run(a)}>
                  {actionText(a)}
                </button>
              )}
            </For>
          </div>
        </section>
      )}
    </Match>
    <Match when={props.section.type === "bio" && props.section}>
      {(s) => (
        <section class="infoview-section" aria-label="Bio">
          <div class="infoview-section-title">Bio</div>
          <div class="infoview-text">{s().text}</div>
        </section>
      )}
    </Match>
    <Match when={props.section.type === "groupMembers" && props.section}>
      {(s) => (
        <section class="infoview-section" aria-label={s().label}>
          <div class="infoview-section-title">{s().label}</div>
          <div class="infoview-meta">
            {s().online.length} online, {s().offline.length} offline
          </div>
          <ul class="infoview-list">
            <For each={[...s().online, ...s().offline]}>
              {(f) => <li class="infoview-row infoview-text">{friendName(f)}</li>}
            </For>
          </ul>
        </section>
      )}
    </Match>
    <Match when={props.section.type === "communityHeader" && props.section}>
      {(s) => (
        <section class="infoview-section" aria-label={s().name}>
          <div class="infoview-section-title">{s().name}</div>
          <Show when={s().description}>
            <div class="infoview-text">{s().description}</div>
          </Show>
          <div class="infoview-meta">
            {s().memberCount} {s().memberCount === 1 ? "member" : "members"}, {s().onlineCount} online
          </div>
          <Show when={s().removed}>
            {(r) => (
              <div class="infoview-meta" role="status">
                {r().banned ? "You were banned" : "You were removed"}
                {r().byName ? ` by ${r().byName}` : ""}
                {r().reason ? `: ${r().reason}` : ""}
              </div>
            )}
          </Show>
        </section>
      )}
    </Match>
    <Match when={props.section.type === "pinned" && props.section}>
      {(s) => (
        <section class="infoview-section" aria-label="Pinned messages">
          <div class="infoview-section-title">Pinned</div>
          <div class="infoview-meta">{s().pins.length} pinned messages</div>
        </section>
      )}
    </Match>
    <Match when={props.section.type === "events" && props.section}>
      {(s) => (
        <section class="infoview-section" aria-label="Events">
          <div class="infoview-section-title">Events</div>
          <Show when={s().events.length > 0} fallback={<div class="infoview-empty">No upcoming events.</div>}>
            <ul class="infoview-list">
              <For each={s().events}>
                {(ev) => (
                  <li class="infoview-row">
                    <span class="infoview-text">{ev.title}</span>
                    <time class="infoview-meta" dateTime={new Date(ev.startTime * 1000).toISOString()}>
                      {new Date(ev.startTime * 1000).toLocaleString()}
                    </time>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </section>
      )}
    </Match>
    <Match when={props.section.type === "gameServers" && props.section}>
      {(s) => (
        <section class="infoview-section" aria-label="Game servers">
          <div class="infoview-section-title">Game Servers</div>
          <ul class="infoview-list">
            <For each={s().servers}>
              {(srv) => (
                <li class="infoview-row">
                  <span class="infoview-text">{srv.label || srv.address}</span>
                  <span class="infoview-meta">{srv.address}</span>
                  <JoinLink action={srv.join} />
                </li>
              )}
            </For>
          </ul>
        </section>
      )}
    </Match>
  </Switch>
);

export default Infoview;
