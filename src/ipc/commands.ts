// Barrel for the Tauri command layer. The DTOs and the `commands`
// invoke-wrapper object were split into the `commands/` subfolder (one
// module per domain) to keep every file under the module size cap;
// call sites continue to use `import { commands, type Foo } from "../ipc/commands"`.

export type { IdentitySummary } from "./commands/dto";
export * from "./commands/types";
export * from "./commands/types_sync";

import { accountCommands } from "./commands/account";
import { communityCommands } from "./commands/community";
import { lobbyCommands } from "./commands/dto_lobby";
import { governanceCommands } from "./commands/governance";
import { groupDmCommands } from "./commands/group_dm";
import { syncCommands } from "./commands/sync";
import { systemCommands } from "./commands/system";
import { voiceCommands } from "./commands/voice";

export type { LobbyRefusal, LobbyVerdict, LobbyView } from "./commands/dto_lobby";
export type { GroupDmMemberView, GroupDmView } from "./commands/group_dm";

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
