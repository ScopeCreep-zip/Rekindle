export type VoiceEvent =
  | {
      // Emitted by backend after local node successfully joins a voice channel.
      // Carries activeCallType so the frontend doesn't decide that locally —
      // fixes C1 (VideoCallPanel never mounted because activeCallType was null).
      type: "localJoined";
      data: { channelId: string; activeCallType: "community" | "dm" };
    }
  | {
      type: "userJoined";
      data: { publicKey: string; displayName: string };
    }
  | { type: "userLeft"; data: { publicKey: string } }
  | {
      type: "userSpeaking";
      data: { publicKey: string; speaking: boolean };
    }
  | {
      type: "userMuted";
      data: { publicKey: string; muted: boolean };
    }
  | {
      type: "connectionQuality";
      data: {
        quality: string;
        rxOverflowDrops: number;
        rxLateDrops: number;
        ingressDrops: number;
      };
    }
  | {
      type: "deviceChanged";
      data: { deviceType: string; deviceName: string; reason: string };
    }
  | {
      // Wave 14 W14.4 — backend tells us audio packets were dropped
      // since the last 1s tick. Frontend may surface as a toast or
      // status-bar indicator so the user sees "audio interrupted"
      // instead of confused silence.
      type: "packetsDropped";
      data: { reason: string; count: number };
    };
