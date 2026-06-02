export type PresenceEvent =
  | { type: "friendOnline"; data: { publicKey: string } }
  | { type: "friendOffline"; data: { publicKey: string } }
  | {
      type: "statusChanged";
      data: {
        publicKey: string;
        status: string;
        statusMessage: string | null;
      };
    }
  | {
      type: "gameChanged";
      data: {
        publicKey: string;
        gameName: string | null;
        gameId: number | null;
        elapsedSeconds: number | null;
        serverAddress: string | null;
      };
    };
