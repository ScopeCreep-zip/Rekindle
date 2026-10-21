import { setDmState } from "../stores/dm.store";
import { commands } from "../ipc/commands";

/// The group list and one group's scrollback from the backend, for the
/// chat window's group mode (architecture §27.2) until E2.6's group DMs
/// replace `list_dms` and `get_dm_messages`.
export async function loadGroupChat(recordKey: string, ownPublicKey: string): Promise<void> {
  const list = await commands.listDms();
  const map: Record<string, (typeof list)[number]> = {};
  for (const conv of list) {
    if (conv.isGroup) map[conv.recordKey] = conv;
  }
  setDmState("conversations", map);
  const rows = await commands.getDmMessages(recordKey, 200);
  setDmState(
    "messages",
    recordKey,
    rows.map((row, idx) => ({
      id: idx + 1,
      senderId: row.senderPseudonym,
      body: row.body,
      timestamp: row.timestamp * 1000,
      isOwn: row.senderPseudonym === ownPublicKey,
    })),
  );
}
