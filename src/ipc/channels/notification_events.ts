/**
 * Device-level notifications, on the `notification-event` channel.
 *
 * These mirror `rekindle_types::subscription_events::NotificationEvent`
 * — the same vocabulary the CLI subscribes to. They replaced a
 * Tauri-only `channels::NotificationEvent`, which meant a CLI could not
 * ring, alert, or prompt without reimplementing the protocol logic
 * behind each one. `callIncoming` in particular was documented as
 * existing *for* a CLI notifier, which a Tauri-only enum prevented.
 *
 * Externally tagged (`{ variantName: {...} }`) rather than
 * `{ type, data }`: the same values cross the daemon IPC as postcard,
 * which cannot decode a tagged enum.
 */
export type NotificationEvent =
  | {
      messageReceived: {
        title: string;
        body: string;
        communityId: string;
        channelId: string;
        /**
         * Resolved per-channel/per-community sound override
         * (architecture §32 Phase 7 Week 25). `null` means the
         * frontend should fall back to its bundled default sound.
         */
        soundRef: string | null;
      };
    }
  | { systemAlert: { title: string; body: string } }
  | { updateAvailable: { version: string } }
  | {
      /**
       * P3.3 — peer requested a Signal session reset. The frontend MUST
       * show a confirmation modal displaying `safetyNumber` for
       * out-of-band verification before invoking
       * `commands.acceptSessionReset`.
       */
      sessionResetRequested: {
        peerPublicKey: string;
        peerDisplayName: string;
        safetyNumber: string;
      };
    }
  | {
      /**
       * Wave 12 W12.3 — sibling to `chat-event::incomingCall`, used as
       * the OS-level / CLI ring channel. Carries `callId` so ringtone
       * start/stop is correlatable to the call's lifecycle.
       */
      callIncoming: {
        callId: string;
        from: string;
        displayName: string;
        kind: "audio" | "video";
        expiresAtMs: number;
        isGroup: boolean;
      };
    };

/** The full subscription event as emitted on the channel. */
export type NotificationSubscriptionEvent = { notification: NotificationEvent };

/**
 * Network attachment state, on the `network-status` channel.
 *
 * Was a standalone `NetworkStatusEvent`; it is now the
 * `attachmentChanged` variant of Tier 1's `NetworkEvent`, which gained
 * `attachmentState` and `hasRoute` to carry what this already had.
 */
export type NetworkEvent =
  | {
      attachmentChanged: {
        attachmentState: string;
        isAttached: boolean;
        publicInternetReady: boolean;
        hasRoute: boolean;
      };
    }
  | { localRoutesDied: { count: number } }
  | { remoteRoutesDied: { peerKeys: string[] } }
  | { watchRenewed: { recordKey: string } }
  | { watchReestablished: { recordKey: string } }
  | { watchFailed: { recordKey: string; error: string } }
  | { valueChanged: { recordKey: string; changedSubkeys: number[] } };

export type NetworkSubscriptionEvent = { network: NetworkEvent };

/** Shape the network indicator reads, whatever variant arrived. */
export interface NetworkStatusEvent {
  attachmentState: string;
  isAttached: boolean;
  publicInternetReady: boolean;
  hasRoute: boolean;
}
