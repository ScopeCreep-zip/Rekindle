/**
 * Architecture §32 a11y — async UI submits (join community, friend
 * request, voice handshake, etc.) sometimes block on Veilid bootstrap
 * or NAT punch retries. Wrap the awaited promise so the UI can surface
 * a user-actionable error instead of spinning forever.
 *
 * The Tauri command itself doesn't currently support cancellation;
 * timing out only stops the UI from awaiting. Any in-flight backend
 * work continues to completion (or silent failure). That's acceptable
 * because the user has moved on by the time the timeout fires.
 */
export class RequestTimeoutError extends Error {
  constructor(label: string, ms: number) {
    super(`${label} timed out after ${(ms / 1000).toFixed(1)}s`);
    this.name = "RequestTimeoutError";
  }
}

export async function withTimeout<T>(
  promise: Promise<T>,
  ms: number,
  label: string,
  signal?: AbortSignal,
): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await new Promise<T>((resolve, reject) => {
      timer = setTimeout(() => reject(new RequestTimeoutError(label, ms)), ms);
      if (signal) {
        if (signal.aborted) {
          reject(new DOMException("Aborted", "AbortError"));
          return;
        }
        signal.addEventListener(
          "abort",
          () => reject(new DOMException("Aborted", "AbortError")),
          { once: true },
        );
      }
      promise.then(resolve, reject);
    });
  } finally {
    if (timer !== undefined) clearTimeout(timer);
  }
}

/** Worst-case Veilid bootstrap window observed in field testing. */
export const JOIN_TIMEOUT_MS = 15_000;
/** DHT writes (governance, channel record). */
export const DHT_WRITE_TIMEOUT_MS = 10_000;
/** 1:1 message dispatch via private route. */
export const MESSAGE_TIMEOUT_MS = 8_000;
