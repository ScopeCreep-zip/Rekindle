// Community voice DTOs (plan E4.1): the Host's answers to `VoiceDevices`
// and `VoiceStatus`, mirroring `rekindle_types::dto::voice`.

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
  | { type: "device"; data: { id: string; name: string; standsInFor: string | null } }
  /** No device could be opened: the Host's reason. */
  | { type: "unavailable"; data: { reason: string } }
  /** Configured without a device (a headless Host). */
  | { type: "headless" };

/** The voice session the Host runs. */
export interface VoiceSessionRef {
  communityId: string;
  channelId: string;
}

/** The Host's voice state. */
export interface VoiceStatus {
  /** `null` when not in a voice channel; the rest then reads idle. */
  session: VoiceSessionRef | null;
  muted: boolean;
  deafened: boolean;
  input: VoiceEndState;
  output: VoiceEndState;
}
