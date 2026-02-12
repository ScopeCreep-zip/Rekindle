import { createStore } from "solid-js/store";

export type UserStatus = "online" | "away" | "busy" | "offline";

export interface GameStatus {
  gameId: number;
  gameName: string;
  elapsedSeconds: number;
}

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
