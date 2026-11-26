// Voice DTOs (plan E4.1): the Host's answers to `VoiceDevices` and
// `VoiceStatus`, mirroring `rekindle_types::dto::voice`. Externally tagged
// like every `VoiceEvent` (the values cross the daemon IPC as postcard,
// which cannot decode a tagged enum).

import type { VoiceScope } from "../channels/voice_events";

/** One audio device in a settings list. */
export interface VoiceDevice {
  /** Stable id: what a choice saves. */
  id: string;
  name: string;
  isDefault: boolean;
  /** Input channels an input-channel choice picks from; 0 for outputs. */
  channels: number;
}

/** The Host's audio devices. */
export interface VoiceDevices {
  inputDevices: VoiceDevice[];
  outputDevices: VoiceDevice[];
}

/** One audio end of the running session (plan E4.1 `AudioIo`, C7.24). */
export type VoiceEndState =
  /** A device is open; `standsInFor` is the saved choice it replaces
   *  while that device is not connected. */
  | { device: { id: string; name: string; standsInFor: string | null } }
  /** No device could be opened: the Host's reason. */
  | { unavailable: { reason: string } }
  /** Configured without a device (a headless Host). */
  | "headless";

/** The Host's voice state: one live session per identity, across 1:1,
 *  group and community calls. */
export interface VoiceStatus {
  /** The live session's scope; `null` when in none (the rest then reads idle). */
  session: VoiceScope | null;
  /** The Host's words for why a new voice session cannot start now
   *  ("Leave the current call first."), or `null` when one can. Every
   *  voice-start button is visibly disabled with it; Leave never is. */
  startRefusal: string | null;
  muted: boolean;
  deafened: boolean;
  input: VoiceEndState;
  output: VoiceEndState;
}
