// What a voice end (the microphone or the speakers) says beside its
// control, from the Host's `VoiceEndState` (plan E4.1 S5, C7.24).
import type { VoiceDevice, VoiceEndState } from "../../ipc/commands/dto_voice";

/** Why the end has no device (the Host's reason), or null when it has one
 *  or is headless. Its control is then visibly disabled, never hidden. */
export function endUnavailable(end: VoiceEndState | undefined): string | null {
  return end !== undefined && typeof end === "object" && "unavailable" in end ? end.unavailable.reason : null;
}

/**
 * The line an end shows: the Host's reason when it has no device, or the
 * device standing in for the saved choice that is not connected ("Using
 * <default> in place of <saved>", the saved device named from the Host's
 * list, or by its id once it is gone from it). Null otherwise.
 */
export function endNotice(end: VoiceEndState | undefined, devices: readonly VoiceDevice[]): string | null {
  if (end === undefined || end === "headless") return null;
  if ("unavailable" in end) return end.unavailable.reason;
  const saved = end.device.standsInFor;
  if (saved === null) return null;
  const name = devices.find((d) => d.id === saved)?.name ?? saved;
  return `Using ${end.device.name} in place of ${name}`;
}
