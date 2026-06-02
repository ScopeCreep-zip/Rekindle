// Barrel for the Tauri command layer. The DTOs and the `commands`
// invoke-wrapper object were split into the `commands/` subfolder (one
// module per domain) to keep every file under the module size cap;
// call sites continue to use `import { commands, type Foo } from "../ipc/commands"`.
export * from "./commands/types";
export * from "./commands/types_sync";

import { accountCommands } from "./commands/account";
import { communityCommands } from "./commands/community";
import { governanceCommands } from "./commands/governance";
import { voiceCommands } from "./commands/voice";
import { systemCommands } from "./commands/system";
import { syncCommands } from "./commands/sync";

export const commands = {
  ...accountCommands,
  ...communityCommands,
  ...governanceCommands,
  ...voiceCommands,
  ...systemCommands,
  ...syncCommands,
};

export function avatarDataUrl(base64: string | null | undefined): string | undefined {
  if (!base64) return undefined;
  return `data:image/webp;base64,${base64}`;
}
