import { createStore } from "solid-js/store";

/** Our status as the Host reports it; `invisible` appears offline to
 *  peers (`SessionStatus`). */
export type UserStatus = "online" | "away" | "busy" | "offline" | "invisible";

// Our own game, as the Host reports it (`dto::presence::OwnGame`).
import type { OwnGame } from "../ipc/commands/dto";

export interface AuthState {
  isLoggedIn: boolean;
  publicKey: string | null;
  displayName: string | null;
  avatarUrl: string | null;
  status: UserStatus;
  statusMessage: string | null;
  gameInfo: OwnGame | null;
}

const [authState, setAuthState] = createStore<AuthState>({
  isLoggedIn: false,
  publicKey: null,
  displayName: null,
  avatarUrl: null,
  status: "offline",
  statusMessage: null,
  gameInfo: null,
});

export { authState, setAuthState };
