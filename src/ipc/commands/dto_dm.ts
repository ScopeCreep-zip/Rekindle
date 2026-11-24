// Direct-message DTOs (plan E2.1, E7.1; `rekindle_types::dto::dm`).
import type { SendState } from "./types";

/** One direct message, as a chat window shows it. */
export interface DmMessage {
  id: number;
  /** Its client id, which a reply names; null for a row from before E2.3. */
  messageId: string | null;
  senderId: string;
  body: string;
  timestamp: number;
  isOwn: boolean;
  sendState: SendState;
  /** A history-gap marker row (plan E2.5, D23): the peer's entries through
   *  this seq were retired before we fetched them; null for a message. */
  historyGapThrough: number | null;
  /** The disappearing timer it carries (plan E7.1; 0 = none). */
  expireSeconds: number;
  /** When the Host deletes it (ms); null with no timer, or while a
   *  received one waits to be displayed. */
  expireAt: number | null;
  /** A timer-change notice row (plan E7.1): `senderId` set the timer to
   *  this many seconds (0 = off); null for a message. */
  timerNoticeSeconds: number | null;
  /** The client id of the message it replies to, as carried (held or
   *  not); `null`: not a reply. */
  replyToId: string | null;
}

/** A conversation's disappearing-message timer as its control shows it
 *  (`disappearing_get`, plan E7.1): the timer in force, the presets to
 *  offer and the longest custom value, all the Host's. */
export interface DisappearingTimerView {
  seconds: number;
  presets: number[];
  maxSeconds: number;
}
