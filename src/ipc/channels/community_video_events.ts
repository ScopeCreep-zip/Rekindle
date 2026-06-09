import type { Codec, ScalabilityMode, SessionVideoConfig } from "../commands/types_sync";

/// Architecture §10.6 — community video / screen-share signalling. Split
/// out of the `CommunityEvent` union (and folded back in via
/// `| CommunityVideoEvent`) so neither module approaches the size cap.
export type CommunityVideoEvent =
  | {
      // Architecture §10.6 — receiver bandwidth ack drives encoder bitrate.
      type: "videoFrameAck";
      data: {
        communityId: string;
        senderPseudonym: string;
        channelId: string;
        streamId: string;
        lastFrameSeq: number;
        kbps: number;
        lossQ8: number;
      };
    }
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
      // Architecture §10.6 — out-of-band bandwidth advertisement.
      type: "videoBandwidthEstimate";
      data: {
        communityId: string;
        senderPseudonym: string;
        channelId: string;
        kbps: number;
        windowSecs: number;
        lossQ8: number;
      };
    }
  | {
      // Architecture §10.6 — peer's decode capabilities for adaptive sender.
      // Phase A — typed `Codec[]` + `supports_optimize_for_latency` +
      // `supported_scalability_modes` (no `Vec<String>` legacy shape).
      type: "videoMediaCapabilities";
      data: {
        communityId: string;
        senderPseudonym: string;
        channelId: string;
        maxPixelCount: number;
        maxFps: number;
        codecs: Codec[];
        supportsOptimizeForLatency: boolean;
        supportedScalabilityModes: ScalabilityMode[];
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
