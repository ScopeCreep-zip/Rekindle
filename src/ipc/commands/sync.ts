import { Channel } from "@tauri-apps/api/core";
import { invoke } from "../invoke";
import type {
  BackgroundSyncReport, CommunityAnalytics, CommunityVideoFrameMsg, DeviceList, DmConversation, DmMessageRecord, DmVideoFrameMsg, LinkPreview, MessageSearch, PairingAccept, PairingQrPayload, PairingSession, SearchResult, SendVideoFrameRequest, SyncManifest, SyncPreferences, SyncReadState, VideoTopologyReason, VideoTrackLabel,
} from "./types_sync";

export const syncCommands = {
  // Direct messages (architecture §27)
  listDms: () => invoke<DmConversation[]>("list_dms"),
  startDm: (bobPublicKey: string, alicePseudonym: string) =>
    invoke<string>("start_dm", { bobPublicKey, alicePseudonym }),
  acceptDmInvite: (recordKey: string) =>
    invoke<void>("accept_dm_invite", { recordKey }),
  declineDmInvite: (recordKey: string) =>
    invoke<void>("decline_dm_invite", { recordKey }),
  sendDmMessage: (recordKey: string, body: string, idempotencyKey?: string) =>
    invoke<void>("send_dm_message", {
      recordKey,
      body,
      // Phase 8 — idempotency key dedupes click-spam.
      idempotencyKey: idempotencyKey ?? crypto.randomUUID(),
    }),
  getDmMessages: (recordKey: string, limit: number) =>
    invoke<DmMessageRecord[]>("get_dm_messages", { recordKey, limit }),
  openDmWindow: (recordKey: string, titleHint: string) =>
    invoke<void>("open_dm_window", { recordKey, titleHint }),

  // Strand Relay Network (architecture §13)
  volunteerRelay: (friendPublicKey: string) =>
    invoke<void>("volunteer_relay", { friendPublicKey }),
  revokeRelay: (friendPublicKey: string) =>
    invoke<void>("revoke_relay", { friendPublicKey }),
  listReceivedRelayOffers: () =>
    invoke<string[]>("list_received_relay_offers"),
  listVolunteeredRelayFriends: () =>
    invoke<string[]>("list_volunteered_relay_friends"),

  // Mobile Push Relay (architecture §17.3)
  registerWithPushRelay: (
    relayPseudonym: string,
    devicePushToken: string,
    platform: "fcm" | "apns" | "self",
    recordKeys: string[],
  ) =>
    invoke<void>("register_with_push_relay", {
      relayPseudonym,
      devicePushToken,
      platform,
      recordKeys,
    }),
  unregisterWithPushRelay: (relayPseudonym: string) =>
    invoke<void>("unregister_with_push_relay", { relayPseudonym }),
  listPushRelayRegistrations: () =>
    invoke<[string, string, string][]>("list_push_relay_registrations"),
  searchMessages: (request: MessageSearch) =>
    invoke<SearchResult>("search_messages", { request }),
  getCommunityAnalytics: (communityId: string) =>
    invoke<CommunityAnalytics>("get_community_analytics", { communityId }),
  ensurePersonalSyncRecord: () => invoke<string>("ensure_personal_sync_record"),
  startPairingSession: () => invoke<PairingSession>("start_pairing_session"),
  generatePairingQrSvg: () => invoke<PairingQrPayload>("generate_pairing_qr_svg"),
  acceptPairingCode: (
    pairingCode: string,
    pairingSaltHex: string,
    existingDeviceRouteBlobHex: string,
    displayName: string,
  ) =>
    invoke<PairingAccept>("accept_pairing_code", {
      pairingCode,
      pairingSaltHex,
      existingDeviceRouteBlobHex,
      displayName,
    }),
  readSyncManifest: () => invoke<SyncManifest | null>("read_sync_manifest"),
  writeSyncManifest: (manifest: SyncManifest) =>
    invoke<void>("write_sync_manifest", { manifest }),
  readSyncReadState: () => invoke<SyncReadState>("read_sync_read_state"),
  writeSyncReadState: (readState: SyncReadState) =>
    invoke<SyncReadState>("write_sync_read_state", { readState }),
  readSyncPreferences: () => invoke<SyncPreferences>("read_sync_preferences"),
  writeSyncPreferences: (preferences: SyncPreferences) =>
    invoke<SyncPreferences>("write_sync_preferences", { preferences }),
  readPairedDevices: () => invoke<DeviceList>("read_paired_devices"),
  writePairedDevices: (devices: DeviceList) =>
    invoke<DeviceList>("write_paired_devices", { devices }),
  fetchLinkPreview: (
    communityId: string,
    channelId: string,
    messageId: string,
    url: string,
  ) =>
    invoke<LinkPreview>("fetch_link_preview", {
      communityId,
      channelId,
      messageId,
      url,
    }),
  runBackgroundSync: () =>
    invoke<BackgroundSyncReport>("run_background_sync"),
  /**
   * Send one VP9-encoded frame chunk (from the WebCodecs VideoEncoder
   * output) into the community video stream. The backend MEK-encrypts,
   * fragments to ≤28 KB, attaches FEC parity for keyframes, signs each
   * fragment, and broadcasts via gossip. Returns the number of
   * fragments dispatched (data + parity).
   */
  sendVideoFrame: (
    communityId: string,
    channelId: string,
    request: SendVideoFrameRequest,
  ) =>
    invoke<number>("send_video_frame", { communityId, channelId, request }),
  /**
   * W11.4 (P6.2) — send one encoded video frame to a 1:1 DM peer.
   * Mirrors `sendVideoFrame` shape but routes through the existing
   * Signal-encrypted DM transport instead of community gossip.
   * Backend chunks ≤28 KB and wraps each chunk in a
   * `DmVideoFragment` payload. Returns the number of fragments sent.
   */
  sendDmVideoFrame: (
    peerPubkey: string,
    request: SendVideoFrameRequest,
  ) =>
    invoke<number>("send_dm_video_frame", { peerPubkey, request }),
  /**
   * Phase 11 Tier 1 — register the per-peer `ipc::Channel` the video
   * panel decodes from. Replaces the high-throughput `dm-video-frame`
   * Tauri event so VP9 frames bypass the shared event bus. The backend
   * pushes each reassembled DM frame straight to `onFrame`; re-registering
   * for the same peer replaces the prior handle.
   */
  registerDmVideoChannel: (
    peerPubkey: string,
    onFrame: Channel<DmVideoFrameMsg>,
  ) =>
    invoke<void>("register_dm_video_channel", { peerPubkey, onFrame }),
  /** Phase 11 Tier 1 — drop the per-peer video channel on panel unmount. */
  unregisterDmVideoChannel: (peerPubkey: string) =>
    invoke<void>("unregister_dm_video_channel", { peerPubkey }),
  /**
   * Derive the deterministic 16-byte stream_id the encoder must stamp
   * into each outbound `VideoFragment`. `trackLabel` distinguishes
   * concurrent streams from the same member — `"camera"` for the
   * webcam track from `getUserMedia()`, `"screen"` for the
   * `getDisplayMedia()` track in the same call (architecture §10.6).
   * Returns lowercase hex.
   */
  deriveVideoStreamId: (
    communityId: string,
    channelId: string,
    trackLabel: VideoTrackLabel,
  ) =>
    invoke<string>("derive_video_stream_id", {
      communityId,
      channelId,
      trackLabel,
    }),
  /**
   * Architecture §10.6 — interim default media capabilities (480p @
   * 15fps, VP9 only) for clients that don't introspect their hardware.
   * The video send-side init in `VoicePanel.tsx::startVideo()` seeds
   * its WebCodecs `VideoEncoderConfig` with these values when the
   * browser's `MediaCapabilities` API isn't available, then advertises
   * the result in `MediaCapabilities` envelopes so peers can size
   * their VP9 bitrate to the slowest receiver.
   */
  defaultMediaCapabilities: () =>
    invoke<{ maxPixelCount: number; maxFps: number; codecs: string[] }>(
      "default_media_capabilities",
    ),
  /**
   * Architecture §10.6 line 4081 — receiver acks frames roughly every
   * 500 ms with measured downstream kbps + loss so senders can adapt
   * VP9 bitrate. `lossQ8` is fixed-point 0..=255 (0 = perfect).
   */
  sendVideoFrameAck: (
    communityId: string,
    channelId: string,
    streamIdHex: string,
    lastFrameSeq: number,
    kbps: number,
    lossQ8: number,
  ) =>
    invoke<void>("send_video_frame_ack", {
      communityId,
      channelId,
      streamIdHex,
      lastFrameSeq,
      kbps,
      lossQ8,
    }),
  /**
   * Architecture §10.6 line 4081 — receiver lost too many inter-frames
   * and asks the sender to mark the next frame as a keyframe.
   */
  sendVideoKeyframeRequest: (
    communityId: string,
    channelId: string,
    streamIdHex: string,
  ) =>
    invoke<void>("send_video_keyframe_request", {
      communityId,
      channelId,
      streamIdHex,
    }),
  /**
   * Architecture §10.6 line 4082 — out-of-band bandwidth advertisement
   * when network conditions change between frames (Wi-Fi → cellular).
   */
  sendVideoBandwidthEstimate: (
    communityId: string,
    channelId: string,
    kbps: number,
    windowSecs: number,
    lossQ8: number,
  ) =>
    invoke<void>("send_video_bandwidth_estimate", {
      communityId,
      channelId,
      kbps,
      windowSecs,
      lossQ8,
    }),
  /**
   * Architecture §10.6 Phase 6 Week 22 — broadcast that the active
   * video relay for `(channelId, streamIdHex)` has changed. Receivers
   * re-attach decoders to the new relay's stream and reset reassembly
   * buffers. `relayHostPseudonym = null` reverts to direct mesh
   * delivery.
   */
  notifyVideoTopologyChange: (
    communityId: string,
    channelId: string,
    streamIdHex: string,
    relayHostPseudonym: string | null,
    reason: VideoTopologyReason,
  ) =>
    invoke<void>("notify_video_topology_change", {
      communityId,
      channelId,
      streamIdHex,
      relayHostPseudonym,
      reason,
    }),
  /**
   * Phase 11 Tier 1 — register the per-community `ipc::Channel` the video
   * panel decodes from. Replaces the `community-event` `videoFrame`
   * variant so high-throughput VP9 frames bypass the shared event bus;
   * control events (acks, keyframe requests, topology) still ride
   * `community-event`. Re-registering replaces the prior handle.
   */
  registerCommunityVideoChannel: (
    communityId: string,
    onFrame: Channel<CommunityVideoFrameMsg>,
  ) =>
    invoke<void>("register_community_video_channel", { communityId, onFrame }),
  /** Phase 11 Tier 1 — drop the per-community video channel on unmount. */
  unregisterCommunityVideoChannel: (communityId: string) =>
    invoke<void>("unregister_community_video_channel", { communityId }),

  // Phase 10 — replay events newer than `lastCursor` (the last cursor
  // this window persisted to localStorage). The backend re-emits each
  // entry **scoped to the calling webview** on its original channel
  // (and a `cursor-tick` per entry to advance localStorage), so
  // multiple windows mounting concurrently never duplicate-process the
  // same backlog. Returns the count of entries replayed — useful for
  // dev-mode diagnostics; the actual events arrive through the live
  // `safeListen` handlers.
  eventResume: (lastCursor: number): Promise<number> =>
    invoke<number>("event_resume", { lastCursor }),
};
