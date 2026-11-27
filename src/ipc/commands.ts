// Barrel for the Tauri command layer. The DTOs and the `commands`
// invoke-wrapper object were split into the `commands/` subfolder (one
// module per domain) to keep every file under the module size cap;
// call sites continue to use `import { commands, type Foo } from "../ipc/commands"`.
export * from "./commands/types";
export type { IdentitySummary } from "./commands/dto";
export * from "./commands/types_sync";

import { accountCommands } from "./commands/account";
import { communityCommands } from "./commands/community";
import { governanceCommands } from "./commands/governance";
import { voiceCommands } from "./commands/voice";
import { systemCommands } from "./commands/system";
import { syncCommands } from "./commands/sync";
import { groupDmCommands } from "./commands/group_dm";
import { lobbyCommands } from "./commands/dto_lobby";
export type { GroupDmView, GroupDmMemberView } from "./commands/group_dm";
export type { LobbyRefusal, LobbyVerdict, LobbyView } from "./commands/dto_lobby";

export const commands = {
  ...accountCommands,
  ...communityCommands,
  ...governanceCommands,
  ...voiceCommands,
  ...systemCommands,
  ...syncCommands,
  ...groupDmCommands,
  ...lobbyCommands,
};
