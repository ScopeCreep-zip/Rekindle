import { invoke } from "../invoke";
import type {
  AudioDevices, MissedCallRow,
} from "./types";

export const voiceCommands = {
  // Voice
  joinVoiceChannel: (channelId: string, communityId?: string) =>
    invoke<void>("join_voice_channel", { channelId, communityId: communityId ?? null }),
  leaveVoice: () => invoke<void>("leave_voice"),
  requestToSpeak: (communityId: string, channelId: string) =>
    invoke<void>("request_to_speak", { communityId, channelId }),
  getStageHandRaises: (communityId: string, channelId: string) =>
    invoke<string[]>("get_stage_hand_raises", { communityId, channelId }),
  respondToSpeakRequest: (communityId: string, channelId: string, requesterPseudonym: string, granted: boolean) =>
    invoke<void>("respond_to_speak_request", { communityId, channelId, requesterPseudonym, granted }),
  setMute: (muted: boolean) => invoke<void>("set_mute", { muted }),
  setDeafen: (deafened: boolean) => invoke<void>("set_deafen", { deafened }),
  /**
   * Architecture §10 — moderator action: server-mute another member
   * in a community voice channel. Backend gates on `MUTE_MEMBERS`
   * and broadcasts a `Control::VoiceMute` envelope through the
   * gossip mesh; receivers honour it locally if the actor still has
   * the perm in CRDT-merged governance.
   */
  serverMuteMember: (
    communityId: string,
    channelId: string,
    targetPseudonym: string,
    muted: boolean,
  ) =>
    invoke<void>("server_mute_member", {
      communityId,
      channelId,
      targetPseudonym,
      muted,
    }),
  /**
   * Architecture §10 — moderator action: server-deafen another
   * member. Backend gates on `DEAFEN_MEMBERS` and broadcasts a
   * `Control::VoiceDeafen` envelope.
   */
  serverDeafenMember: (
    communityId: string,
    channelId: string,
    targetPseudonym: string,
    deafened: boolean,
  ) =>
    invoke<void>("server_deafen_member", {
      communityId,
      channelId,
      targetPseudonym,
      deafened,
    }),
  listAudioDevices: () => invoke<AudioDevices>("list_audio_devices"),
  setAudioDevices: (inputDevice: string | null, outputDevice: string | null) =>
    invoke<void>("set_audio_devices", { inputDevice, outputDevice }),
  setVoiceMode: (mode: string, hostPseudonym?: string) =>
    invoke<void>("set_voice_mode", { mode, hostPseudonym: hostPseudonym ?? null }),

  // Plan §Failure 5 — direct call offer/accept handshake.
  startDmCall: (peerPublicKey: string, video: boolean) =>
    invoke<string>("start_dm_call", { peerPublicKey, video }),
  acceptDmCall: (callId: string) =>
    invoke<void>("accept_dm_call", { callId }),
  declineDmCall: (callId: string, reason?: string) =>
    invoke<void>("decline_dm_call", { callId, reason: reason ?? null }),
  /// C2 hangup — end an Active call (post-handshake), distinct from
  /// declineDmCall (which rejects a CallOffer before accepting).
  endDmCall: (callId: string, reason?: string) =>
    invoke<void>("end_dm_call", { callId, reason: reason ?? null }),
  /// Wave 12 W12.6 — fire-and-forget mid-call media-state ping so the
  /// peer's UI mounts/unmounts video and screen-share tiles in sync
  /// with our local toggle.
  sendCallMediaState: (
    callId: string,
    audio: boolean,
    video: boolean,
    screen: boolean,
  ) =>
    invoke<void>("send_call_media_state", { callId, audio, video, screen }),
  /// Wave 12 W12.12 — temporarily silence a caller. Future incoming
  /// offers from `peerPublicKey` auto-decline with "user is
  /// unavailable" without ringing until `durationMs` elapses. Cleared
  /// on backend restart (in-memory only).
  muteCallerTemp: (peerPublicKey: string, durationMs: number) =>
    invoke<void>("mute_caller_temp", { peerPublicKey, durationMs }),
  /// Wave 12 W12.11 — fire an in-call emoji reaction at the peer.
  /// Best-effort; loss is tolerable.
  sendCallReaction: (callId: string, emoji: string) =>
    invoke<void>("send_call_reaction", { callId, emoji }),
  /// Wave 12 W12.9 — group calls. Initiator fans out a per-recipient
  /// wrapped call_key to every invitee.
  startGroupCall: (participantPubkeys: string[], video: boolean) =>
    invoke<string>("start_group_call", { participantPubkeys, video }),
  acceptGroupCall: (callId: string) =>
    invoke<void>("accept_group_call", { callId }),
  declineGroupCall: (callId: string, reason?: string) =>
    invoke<void>("decline_group_call", { callId, reason: reason ?? null }),
  endGroupCall: (callId: string, reason?: string) =>
    invoke<void>("end_group_call", { callId, reason: reason ?? null }),
  /// B6 — explicit user-driven Signal session reset. Surfaced from the
  /// friend context menu when the user has verified the peer's safety
  /// number out-of-band and wants to re-handshake. NOT auto-invoked on
  /// decrypt failure (vulnerable-user safety stance forbids
  /// auto-rehandshake).
  ///
  /// P3.3 — also sends a SessionResetRequest to the peer carrying our
  /// fresh PreKeyBundle. Peer's UI prompts them to verify our safety
  /// number before accepting; on accept they reply with
  /// SessionResetAccept which our backend processes to install the new
  /// responder-side session.
  resetSignalSession: (peerPublicKey: string) =>
    invoke<void>("reset_signal_session", { peerPublicKey }),
  /// P3.3 — accept a SessionResetRequest after verifying the peer's
  /// safety number out-of-band. Frontend MUST gate this behind explicit
  /// user consent (modal showing safety_number from
  /// NotificationEvent::SessionResetRequested).
  acceptSessionReset: (peerPublicKey: string) =>
    invoke<void>("accept_session_reset", { peerPublicKey }),
  /// P3.3 — decline a SessionResetRequest.
  declineSessionReset: (peerPublicKey: string, reason?: string) =>
    invoke<void>("decline_session_reset", {
      peerPublicKey,
      reason: reason ?? null,
    }),
  getMissedCalls: () => invoke<MissedCallRow[]>("get_missed_calls"),
};
