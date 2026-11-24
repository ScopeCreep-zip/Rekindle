import { hostInvoke } from "../invoke";
import type { TransferTarget } from "./dto_transfer";
import type { SearchHit } from "./types_sync";

/** Where a local search looks (`rekindle_types::dto::local_search::LocalSearchScope`, plan E7.2 G11). */
export type LocalSearchScope =
  | { kind: "friends" }
  | { kind: "servers" }
  | { kind: "files" }
  | { kind: "conversation"; arg: { conversationId: string } };

/** One local search hit (`LocalHit`): what it names and what opening it opens. */
export type LocalHit =
  | { kind: "friend"; arg: { publicKey: string; name: string } }
  | { kind: "groupDm"; arg: { groupId: string; name: string } }
  | { kind: "member"; arg: { community: string; pseudonym: string; name: string } }
  /** `label` is "" for a recent server (no saved name). */
  | { kind: "server"; arg: { gameId: number; address: string; label: string } }
  | { kind: "file"; arg: { attachmentId: string; target: TransferTarget; name: string } }
  | { kind: "message"; arg: SearchHit };

/** `LocalSearchResult`: the hits, best first. */
export interface LocalSearchResult {
  hits: LocalHit[];
}

export const localSearchCommands = {
  /** Search this device's own data in `scope` for `query` (SQLite only, `IpcRequest::LocalSearch`). */
  search: (scope: LocalSearchScope, query: string) => hostInvoke<LocalSearchResult>("local_search", { scope, query }),
};
