/**
 * Send states the Host announced (`ChannelSendStatus`) before our own row knew
 * its message id (FT1: the status may beat the send's own answer), held by id
 * until the row takes them. Only our own posts are announced; one never
 * matched (its window closed) must not be held forever, so the oldest goes
 * past a bound.
 */

/** A held send state: the Host's state word and, for a failed send, its reason. */
interface EarlySendState {
  state: string;
  reason?: string;
}

/** By message id, insertion-ordered. */
const earlySendStates = new Map<string, EarlySendState>();

/** How many unmatched send states are held: far above the replies one window has in flight. */
const EARLY_SEND_STATES_MAX = 256;

/** Hold `messageId`'s newest send state until its row can take it. */
export function holdEarlySendState(messageId: string, state: string, reason?: string): void {
  earlySendStates.delete(messageId);
  earlySendStates.set(messageId, { state, reason });
  if (earlySendStates.size > EARLY_SEND_STATES_MAX) {
    const oldest = earlySendStates.keys().next().value;
    if (oldest !== undefined) earlySendStates.delete(oldest);
  }
}

/**
 * The newest send state held for `messageId`, taken once; undefined when
 * none came before its row.
 */
export function takeEarlySendState(messageId: string): EarlySendState | undefined {
  const early = earlySendStates.get(messageId);
  earlySendStates.delete(messageId);
  return early;
}
