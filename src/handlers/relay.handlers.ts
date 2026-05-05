import { commands } from "../ipc/commands";
import { relayState, setRelayState } from "../stores/relay.store";

/// Hydrate the relay store from SQLite — both the friends we relay for
/// and the friends who've volunteered to relay for us. Called on
/// buddy-list mount.
export async function handleHydrateRelayState(): Promise<void> {
  const [volunteered, received] = await Promise.all([
    commands.listVolunteeredRelayFriends(),
    commands.listReceivedRelayOffers(),
  ]);
  const vMap: Record<string, true> = {};
  for (const k of volunteered) vMap[k] = true;
  setRelayState("volunteeredFor", vMap);
  const rMap: Record<string, true> = {};
  for (const k of received) rMap[k] = true;
  setRelayState("receivedOffersFrom", rMap);
}

export async function handleVolunteerRelay(friendPublicKey: string): Promise<void> {
  await commands.volunteerRelay(friendPublicKey);
  setRelayState("volunteeredFor", friendPublicKey, true);
}

export async function handleRevokeRelay(friendPublicKey: string): Promise<void> {
  await commands.revokeRelay(friendPublicKey);
  setRelayState("volunteeredFor", friendPublicKey, undefined!);
}

export { relayState };
