/**
 * Voice events, as they arrive on the `voice-event` channel.
 *
 * These mirror `rekindle_types::subscription_events::VoiceEvent` — the
 * same vocabulary the CLI subscribes to. They replaced a Tauri-only
 * `channels::VoiceEvent` that held the *local session's* state (device,
 * speaking, quality, drops) while Tier 1 held only what gossip said
 * about a community channel, so neither could describe a whole call.
 *
 * Shapes are pinned on the Rust side by
 * `json_shape_is_what_the_webview_parses`.
 *
 * Externally tagged (`{ variantName: {...} }`) rather than
 * `{ type, data }`: the same values cross the daemon IPC as postcard,
 * which cannot decode a tagged enum.
 */

/**
 * Where a call is happening.
 *
 * A DM call previously had no representation at all — every variant
 * required a community — so it was smuggled through a separate
 * `activeCallType: "dm"` string. It is now a scope of its own.
 */
export type VoiceScope =
  | { community: { community: string; channel: string } }
  | { dm: { peerKey: string } };

export type VoiceEvent =
  | { joined: { scope: VoiceScope; pseudonym: string; displayName: string | null } }
  | { left: { scope: VoiceScope; pseudonym: string } }
  | { modeChanged: { scope: VoiceScope; mode: string; hostPseudonym: string | null } }
  | { muteChanged: { scope: VoiceScope; targetPseudonym: string; muted: boolean } }
  | { deafenChanged: { scope: VoiceScope; targetPseudonym: string; deafened: boolean } }
  | { rosterUpdated: { scope: VoiceScope; participantCount: number } }
  /** **We** joined a call and the audio pipeline is running. */
  | { localJoined: { scope: VoiceScope } }
  | { speakingChanged: { scope: VoiceScope; pseudonym: string; speaking: boolean } }
  /** Machine-wide — no scope, since a device can change with no call. */
  | { deviceChanged: { deviceType: string; deviceName: string; reason: string } }
  | { packetsDropped: { scope: VoiceScope; reason: string; count: number } }
  | {
      connectionQuality: {
        scope: VoiceScope;
        quality: string;
        rxOverflowDrops: number;
        rxLateDrops: number;
        /** Inbound media dropped for MEK reasons (rotation race signal). */
        rxMekDrops: number;
        ingressDrops: number;
      };
    };

/** The full subscription event as emitted on the channel. */
export type VoiceSubscriptionEvent = { voice: VoiceEvent };

/** `"community"` or `"dm"` — what the UI switches its call panel on. */
export function callType(scope: VoiceScope): "community" | "dm" {
  return "community" in scope ? "community" : "dm";
}

/** Channel id for a community call, peer key for a DM. */
export function sessionKey(scope: VoiceScope): string {
  return "community" in scope ? scope.community.channel : scope.dm.peerKey;
}

/** The community, or `null` for a DM call. */
export function scopeCommunity(scope: VoiceScope): string | null {
  return "community" in scope ? scope.community.community : null;
}
