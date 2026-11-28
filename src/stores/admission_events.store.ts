/**
 * The order of CP1 admission events against history reads. A read that began
 * before an admission event for a post may answer after it, carrying the
 * post's older admission; the event's row is the newer one, so the read must
 * not restore the older. Each event takes the next sequence number for its
 * post; a read notes the sequence when it begins, and a post whose event came
 * after that keeps the event's row (by order, not by which states may follow
 * which).
 */

/** The newest event sequence by message id, insertion-ordered. */
const eventSeq = new Map<string, number>();

let seq = 0;

/** How many posts' event order is held: far above the posts a window can show. */
const EVENT_ORDER_MAX = 1024;

/** An admission event for `messageId` arrived: it is newer than every read begun so far. */
export function noteAdmissionEvent(messageId: string): void {
  seq += 1;
  eventSeq.delete(messageId);
  eventSeq.set(messageId, seq);
  if (eventSeq.size > EVENT_ORDER_MAX) {
    const oldest = eventSeq.keys().next().value;
    if (oldest !== undefined) eventSeq.delete(oldest);
  }
}

/** The mark a history read takes when it begins. */
export function readMark(): number {
  return seq;
}

/** Whether `messageId` had an admission event after the read that took `mark` began. */
export function eventAfter(messageId: string | undefined, mark: number): boolean {
  return messageId !== undefined && (eventSeq.get(messageId) ?? 0) > mark;
}
