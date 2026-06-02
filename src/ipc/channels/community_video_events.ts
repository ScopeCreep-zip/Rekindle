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
      type: "videoMediaCapabilities";
      data: {
        communityId: string;
        senderPseudonym: string;
        channelId: string;
        maxPixelCount: number;
        maxFps: number;
        codecs: string[];
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
