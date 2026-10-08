import { createSignal } from "solid-js";

/// A peer's request to re-establish the secure session, waiting for the
/// user's decision (`NotificationEvent::SessionResetRequested`).
export interface SessionResetRequest {
  peerPublicKey: string;
  peerDisplayName: string;
  safetyNumber: string;
}

const [sessionResets, setSessionResets] = createSignal<SessionResetRequest[]>([]);

/// Add a request; a newer one from the same peer replaces the older.
export function queueSessionReset(request: SessionResetRequest): void {
  setSessionResets((prev) => [
    ...prev.filter((r) => r.peerPublicKey !== request.peerPublicKey),
    request,
  ]);
}

/// Remove the request once the user has answered it.
export function resolveSessionReset(peerPublicKey: string): void {
  setSessionResets((prev) => prev.filter((r) => r.peerPublicKey !== peerPublicKey));
}

export { sessionResets };
