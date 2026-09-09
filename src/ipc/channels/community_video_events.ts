import type { Codec, ScalabilityMode, SessionVideoConfig } from "../commands/types_sync";

/// Architecture §10.6 — community video / screen-share signalling. Split
/// out of the `CommunityEvent` union (and folded back in via
/// `| CommunityVideoEvent`) so neither module approaches the size cap.
export type CommunityVideoEvent =
  | {
      // Architecture §10.6 — receiver requests an I-frame.
      type: "videoKeyframeRequest";
      data: {
        communityId: string;
        senderPseudonym: string;
        channelId: string;
        streamId: string;
      };
    }
  | {
      // Architecture §10.6 Phase B — backend-negotiated per-call
      // `SessionVideoConfig`. Recomputed on every join/leave/cap-receipt
      // and re-emitted whenever the negotiated shape changes. The
      // frontend tears down + reconfigures encoder + decoder against
      // the new constraints. Backend owns the policy — CLI / TUI
      // frontends inherit the same payload.
      type: "videoSessionConfig";
      data: {
        communityId: string;
        channelId: string;
        config: SessionVideoConfig;
      };
    }
  | {
      // Phase 4 — backend bitrate policy output (AIMD over receiver
      // feedback, audio reserve subtracted). The encoder follows this
      // target; the backend pacer rate moves with it.
      type: "videoBitrateTarget";
      data: {
        communityId: string;
        channelId: string;
        kbps: number;
      };
    }
  | {
      // The Linux-native capture session died asynchronously (camera
      // unplug, pipeline failure) — revert the toggle + surface it.
      type: "nativeVideoError";
      data: {
        communityId: string;
        channelId: string;
        message: string;
      };
    }
  | {
      // Phase 3 — the backend negotiator found NO local encode codec
      // every peer can decode. Latched backend-side: fires once per
      // compatible→incompatible transition. `peers` lists the blocking
      // pseudonyms. Voice is unaffected.
      type: "videoCodecIncompatible";
      data: {
        communityId: string;
        channelId: string;
        peers: string[];
      };
    }
  | {
      // Phase F — a gossiped video envelope failed signature or shape
      // verification at the receive boundary. Surfaced so the
      // asymmetric-drop case (one peer rejects, the other doesn't) is
      // observable from frontend signals.
      type: "videoEnvelopeRejected";
      data: {
        communityId: string;
        senderPseudonym: string;
        reason: string;
      };
    }
  | {
      // Architecture §10.6 + §22 — relay change (full mesh ↔ SFU).
      // `lamport` is the sender's per-community Lamport clock at the
      // moment of the topology decision; the reassembler uses it to
      // resolve simultaneous topology changes from multiple peers
      // (highest lamport wins, with sender_pseudonym as tiebreaker).
      type: "videoTopologyChange";
      data: {
        communityId: string;
        senderPseudonym: string;
        channelId: string;
        streamId: string;
        relayHostPseudonym: string | null;
        reason: string;
        lamport: number;
      };
    };
