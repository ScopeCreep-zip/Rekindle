import { createStore } from "solid-js/store";

export type UserStatus = "online" | "away" | "busy" | "offline";

// Declared once in the IPC layer. This copy was missing `serverInfo`,
// so typed code here could not see a field the backend already sends.
import type { GameStatus } from "../ipc/commands/types";
export type { GameStatus };

export interface AuthState {
  isLoggedIn: boolean;
  publicKey: string | null;
  displayName: string | null;
  avatarUrl: string | null;
  status: UserStatus;
  statusMessage: string | null;
  gameInfo: GameStatus | null;
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
