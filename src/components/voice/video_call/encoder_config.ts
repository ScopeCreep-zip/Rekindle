// Encoder negotiation and configuration — the pure (closure-free) half
// of the send path, extracted from video_sender.ts. Everything here is
// a function of its arguments plus the video store / capability probe;
// nothing touches the running encoder.

import { commands } from "../../../ipc/commands";
import type { Codec, MediaCapabilities, SessionVideoConfig } from "../../../ipc/commands";
import { localVideoCapabilities } from "../../../handlers/video.handlers";
import { dmPeerDecodeCodecsFor, videoSessionConfigFor } from "../../../stores/video.store";
import { wireCodecToWebCodecsString } from "./codec_utils";
import type { SenderRoute } from "./sender_types";

/** Pure encodability gate: the encoder must never be configured with a
 *  codec the local engine can't encode. `null` caps = probe hasn't
 *  reported — equally not configurable. */
export function pickEncoderCodec(
  local: MediaCapabilities | null,
  negotiated: Codec,
): { ok: boolean; reason?: string } {
  if (!local) {
    return { ok: false, reason: "local capabilities not probed yet" };
  }
  if (!local.encodeCodecs.includes(negotiated)) {
    return {
      ok: false,
      reason: `negotiated codec ${negotiated} not locally encodable (have: ${
        local.encodeCodecs.join(",") || "none"
      })`,
    };
  }
  return { ok: true };
}

/** Phase 5 — first of OUR probed encode codecs the DM peer can
 *  decode (their list rides CallInvite/CallAccept). An empty peer
 *  list means "unknown" (pre-fetch race / pre-probe peer): the vp9
 *  wire floor applies, but ONLY when we can actually encode it —
 *  otherwise wait (`null`); the store write from the caps fetch
 *  re-runs this via the pump. */
export function pickDmEncoderCodec(peerId: string): Codec | null {
  const local = localVideoCapabilities();
  if (!local) return null;
  const peerDecode = dmPeerDecodeCodecsFor(peerId);
  if (!peerDecode || peerDecode.length === 0) {
    return local.encodeCodecs.includes("vp9") ? "vp9" : null;
  }
  return local.encodeCodecs.find((c) => peerDecode.includes(c)) ?? null;
}

/** The negotiated encoder constraints, or `null` when no encodable
 *  shape exists YET (community: config not in store — only possible
 *  mid-call during renegotiation, the media-ready gate guarantees a
 *  config before camera start; DM: caps/peer-list race). The pump
 *  idles on `null` and picks up the store write on a later tick. */
export function encoderConstraints(route: SenderRoute): SessionVideoConfig["encoder"] | null {
  if (route.mode === "community") {
    return videoSessionConfigFor(route.communityId, route.channelId)?.encoder ?? null;
  }
  const codec = pickDmEncoderCodec(route.peerId);
  if (codec === null) return null;
  return {
    codec,
    maxWidth: 854,
    maxHeight: 480,
    maxFps: 15,
    scalabilityMode: "flat",
  };
}

/** Build the WebCodecs config for `encoder.configure()`. `bitrate` is
 *  rebound per-call because the adaptive loop varies it independently
 *  of the negotiated capability shape. H.264 encodes Annex-B so
 *  SPS/PPS ride the bitstream — receivers configure their decoder
 *  codec-string-only, no avcC `description` plumbing. */
export function buildEncoderConfig(
  constraints: SessionVideoConfig["encoder"],
  bitrate: number,
  shape?: { width: number; height: number; fps: number },
): VideoEncoderConfig {
  const base: VideoEncoderConfig = {
    codec: wireCodecToWebCodecsString(constraints.codec),
    width: shape?.width ?? constraints.maxWidth,
    height: shape?.height ?? constraints.maxHeight,
    framerate: shape?.fps ?? constraints.maxFps,
    bitrate,
    latencyMode: "realtime",
    ...(constraints.codec === "h264" ? { avc: { format: "annexb" as const } } : {}),
  };
  return constraints.scalabilityMode === "l1t2" ? { ...base, scalabilityMode: "L1T2" } : base;
}

/** Encoder bitrate coupled to the EFFECTIVE fps. Under CBR,
 *  per-frame bytes = bitrate ÷ fps — handing the full AIMD target
 *  to a low-fps stream concentrates the whole budget into a few
 *  giant frames (live: 1200 kbps at 2 fps = 75 KB average frames,
 *  ~190 KB keyframes via libvpx's 250% max-intra, each costing
 *  seconds of pacer drain — the frozen-tile chain). Scaling by
 *  fps/5 bounds a keyframe to ~½ s of pacer budget at any level;
 *  at ≥5 fps the full target applies. Floor keeps the encoder out
 *  of its degenerate sub-50 kbps range. */
export function effectiveBitrate(kbps: number, fps: number): number {
  return Math.max(50_000, Math.round(kbps * 1000 * Math.min(1, fps / 5)));
}

/** Backend log sink for encoder lifecycle events — the WKWebView /
 *  WebKitGTK divergence must be visible in `RUST_LOG` traces, not
 *  only in a devtools console nobody has open. */
export function reportEncoderStatus(
  route: SenderRoute,
  codec: Codec,
  ok: boolean,
  detail: string,
): void {
  void commands
    .reportVideoEncoderStatus(
      route.mode === "community" ? route.communityId : null,
      route.mode === "dm" ? route.peerId : null,
      codec,
      ok,
      detail,
    )
    .catch(() => {
      // Logging side-channel only — never disturb the pipeline.
    });
}
