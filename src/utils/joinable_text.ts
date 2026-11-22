import type { NotJoinableReason } from "../ipc/commands/types";

/** What the Host states about joining someone's game (plan E7.15 IV3c). */
export interface Joinability {
  joinable: boolean;
  joinableReason: NotJoinableReason | null;
}

/**
 * Why `game` cannot be joined, from the Host's `joinableReason`, as the
 * lower-case phrase a disabled Join Game shows beside it; `game` null is
 * someone not in a game. Empty while it can be joined.
 */
export function notJoinableText(game: Joinability | null): string {
  if (!game) return "not in a game";
  if (game.joinable) return "";
  switch (game.joinableReason) {
    case "noServer":
      return "not on a server";
    case "cannotLaunch":
      return "can't be launched here";
    default:
      return "";
  }
}
