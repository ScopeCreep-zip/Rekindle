import { hostInvoke } from "../invoke";

/** A group's game lobby state (`rekindle_types::dto::lobby::LobbyState`, plan E7.2 G6). */
type LobbyState = { kind: "open" } | { kind: "launched"; server: string } | { kind: "closed" };

/** Why the Host refuses a lobby action (`LobbyRefusal`). */
export type LobbyRefusal = "notOpen" | "closed" | "ownLobby" | "notHost" | "alreadyIn" | "notIn" | "noMembers";

/** The Host's verdict on one lobby action: always shown, disabled with its reason when refused. */
export type LobbyVerdict = { allowed: true } | { allowed: false; reason: LobbyRefusal };

/** `LobbyView`: a group's lobby as the Host composes it, with each action's verdict. */
export interface LobbyView {
  lobbyId: string;
  hostKey: string;
  gameId: number;
  /** Members' identity keys, the host among them. */
  members: string[];
  state: LobbyState;
  join: LobbyVerdict;
  leave: LobbyVerdict;
  launch: LobbyVerdict;
  close: LobbyVerdict;
}

export const lobbyCommands = {
  /** Open a lobby in `groupId` for the game we are shown in (the Host picks it). */
  lobbyHost: (groupId: string) => hostInvoke<LobbyView>("lobby_host", { groupId }),
  lobbyJoin: (groupId: string, lobbyId: string) => hostInvoke<LobbyView>("lobby_join", { groupId, lobbyId }),
  lobbyLeave: (groupId: string, lobbyId: string) => hostInvoke<LobbyView | null>("lobby_leave", { groupId, lobbyId }),
  lobbyClose: (groupId: string, lobbyId: string) => hostInvoke<LobbyView | null>("lobby_close", { groupId, lobbyId }),
  /** Launch for every member (TrustChanging: the review names the group and host:port); null lets the Host take our server. */
  lobbyLaunch: (groupId: string, lobbyId: string, server: string | null) =>
    hostInvoke<LobbyView>("lobby_launch", { groupId, lobbyId, server }),
  lobbyGet: (groupId: string) => hostInvoke<LobbyView | null>("lobby_get", { groupId }),
};
