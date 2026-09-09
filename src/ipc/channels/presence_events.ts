/**
 * Presence events, as they arrive on the `presence-event` channel.
 *
 * These types mirror `rekindle_types::subscription_events::PresenceEvent`
 * — the same vocabulary the CLI subscribes to over the daemon socket.
 * They replaced a Tauri-only `channels::presence_channel::PresenceEvent`
 * that carried a different set of fields, so the two frontends observed
 * different events for the same action.
 *
 * The shapes below are pinned on the Rust side by
 * `json_shape_is_what_the_webview_parses`, so a rename there fails a test
 * rather than silently breaking this file.
 *
 * The enums are **externally tagged** (`{ variantName: {...} }`) rather
 * than the `{ type, data }` used elsewhere: the same values also cross
 * the daemon IPC as postcard, which cannot decode a tagged enum.
 */

/** What a peer is playing, when the emitter actually looked. */
export type GameActivity =
  | "idle"
  | {
      playing: {
        gameName: string;
        gameId: number | null;
        elapsedSeconds: number | null;
        serverAddress: string | null;
      };
    };

/**
 * One observation of a peer's presence.
 *
 * `null` means **not observed**, never "cleared" — the game scanner
 * reports a game and no status, the idle timer reports a status and no
 * game. Apply only the fields that are present; a consumer that
 * overwrites on `null` will wipe a running game every time the peer's
 * status is restated.
 *
 * "Observed, and stopped playing" is `game: "idle"`, which is distinct
 * from `game: null`.
 */
export interface PresenceSnapshot {
  status: string | null;
  statusMessage: string | null;
  game: GameActivity | null;
}

export type PresenceEvent =
  | {
      communityMemberChanged: {
        community: string;
        pseudonym: string;
        snapshot: PresenceSnapshot;
      };
    }
  | { friendChanged: { peerKey: string; snapshot: PresenceSnapshot } }
  | { selfChanged: { publicKey: string; snapshot: PresenceSnapshot } };

/** The full subscription event as emitted on every channel. */
export type PresenceSubscriptionEvent = { presence: PresenceEvent };

/** The game name, or `null` when idle or unobserved. */
export function gameName(snapshot: PresenceSnapshot): string | null {
  const game = snapshot.game;
  if (game === null || game === "idle") return null;
  return game.playing.gameName;
}

/** Whether a game was observed at all (running or stopped). */
export function gameObserved(snapshot: PresenceSnapshot): boolean {
  return snapshot.game !== null;
}
