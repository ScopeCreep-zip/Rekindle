/**
 * The words of an intent review (ADR 0019): the action as a verb and its
 * object, the subject, and each typed fact and escalation the Host states.
 * Pure: the Host decides what the review says; this only words it.
 */

import type {
  Escalation, Fact, IntentKind, IntentReview, Notifies, IrreversibleReason, Recipient, Reversibility, Subject,
} from "../ipc/commands/dto";
import { groupDigits } from "./safety_number";

/** A kind whose review words are not its name's: the contract's wording. */
const KIND_WORDS: Partial<Record<IntentKind, string>> = {
  // E3.4 S6 contract: "Change your name in this community".
  setCommunityDisplayName: "Change your name in this community",
};

/** `deleteChannel` → `Delete channel`, unless the kind has its own words. */
export function kindPhrase(kind: IntentKind): string {
  const own = KIND_WORDS[kind];
  if (own) return own;
  const words = kind.replace(/([a-z0-9])([A-Z])/g, "$1 $2").toLowerCase();
  return words.charAt(0).toUpperCase() + words.slice(1);
}

/** Who or what the review is about. A person leads with our nickname and
 *  the key fingerprint; a peer-chosen name is never shown alone. */
export function subjectLabel(subject: Subject): string {
  if ("person" in subject) {
    const p = subject.person;
    const name = p.localNickname ?? p.displayName;
    return name === null ? p.fingerprint : `${name} (${p.fingerprint})`;
  }
  if ("community" in subject) return subject.community.name;
  if ("channel" in subject) return `#${subject.channel.name} in ${subject.channel.communityName}`;
  if ("role" in subject) return subject.role.name;
  if ("file" in subject) return subject.file.name;
  if ("link" in subject) return subject.link.host;
  if ("ownIdentity" in subject) return subject.ownIdentity.name;
  return subject.named.name;
}

/** What a typed proof must match: the object's own name. */
export function typedName(subject: Subject): string {
  // The full fingerprint, never a nickname (`Subject::typed_name`, finding 79).
  if ("person" in subject) return subject.person.fingerprint;
  if ("community" in subject) return subject.community.name;
  if ("channel" in subject) return subject.channel.name;
  if ("role" in subject) return subject.role.name;
  if ("file" in subject) return subject.file.name;
  if ("link" in subject) return subject.link.host;
  if ("ownIdentity" in subject) return subject.ownIdentity.name;
  return subject.named.name;
}

/**
 * The final button, never "OK". Where pressing it is the whole proof
 * (`commit`, ADR 0019's Consequential class: "a button named verb +
 * object"), it names the verb and its object. Where the proof is a ticked
 * fact, the typed name or the passphrase, the review's heading and subject
 * line already name the object, so the button is the verb alone and does
 * not wrap in a narrow window (walk B1, 2026-11-21).
 */
export function commitLabel(review: IntentReview): string {
  const verb = kindPhrase(review.kind);
  return review.proof === "commit" ? `${verb}: ${subjectLabel(review.subject)}` : verb;
}

/** Who our IP address reaches: a game server by its game and address; a
 *  link's site as "The site" (the subject line already names its host). */
function recipientText(to: Recipient): string {
  if ("gameServer" in to) return `${to.gameServer.game}'s server at ${to.gameServer.address}`;
  return "The site";
}

function reasonText(reason: IrreversibleReason): string {
  const party = reason.disclosureCannotBeRecalled.to === "gameServer" ? "server" : "site";
  return `Your IP address can't be taken back once that ${party} has seen it.`;
}

export function factText(fact: Fact): string {
  if (fact === "subjectNotTold") return "They are not told.";
  if (fact === "pendingMessagesDropped") return "Messages waiting from them are dropped.";
  if (fact === "sessionDeleted") return "Your secure session with them is deleted.";
  if (fact === "sessionKeptUntilReplaced")
    return "Your secure session with them stays until the new one their answer opens replaces it.";
  if (fact === "unopenedMessagesLost")
    return "Messages from them that could not be opened are lost unless they send them again.";
  if (fact === "presenceVisible") return "They will see when you are online and can reach you.";
  if (fact === "rejoinNeedsInvite") return "Rejoining needs a new invite.";
  if (fact === "noHistoryCopied") {
    return "No history is copied; this community stays readable.";
  }
  if ("ipAddressLeaves" in fact) return `${recipientText(fact.ipAddressLeaves.to)} will see your IP address.`;
  if ("newCreatorSet" in fact) {
    return `The new community's creators: ${fact.newCreatorSet.creators.join(", ")}.`;
  }
  if ("creatorsLeave" in fact) {
    const leaving = fact.creatorsLeave.creators;
    return leaving.length === 0
      ? "No creator leaves the creator set."
      : `These creators are not creators of the new community: ${leaving.join(", ")}.`;
  }
  if ("transferLinksMissing" in fact) {
    return `Not linked yet: ${fact.transferLinksMissing.creators.join(", ")}. The transfer completes once every creator has linked.`;
  }
  if ("successorLinked" in fact) {
    return `Signed by ${fact.successorLinked.signer}; both communities link to each other.`;
  }
  if ("fingerprint" in fact) return `Key fingerprint: ${fact.fingerprint.fingerprint}`;
  if ("safetyNumber" in fact) {
    return `Safety number: ${groupDigits(fact.safetyNumber.safetyNumber).join(" ")}`;
  }
  if ("safetyNumberChanged" in fact) {
    const { oldSafetyNumber, newSafetyNumber } = fact.safetyNumberChanged;
    return `Their key changed. Safety number was ${groupDigits(oldSafetyNumber).join(" ")}; ` +
      `it becomes ${groupDigits(newSafetyNumber).join(" ")}.`;
  }
  if ("keyChanged" in fact) {
    const { oldFingerprint, newFingerprint } = fact.keyChanged;
    return `Warning: their key changed. It was ${oldFingerprint}; it becomes ${newFingerprint}. ` +
      "Confirming does not verify it.";
  }
  if ("membersAffected" in fact) return `${fact.membersAffected.count} members are affected.`;
  return `${fact.messagesDeleted.count} messages are deleted for members.`;
}

export function escalationText(escalation: Escalation): string {
  if (escalation === "firstMediaInCall") return "This is the first time your camera or screen goes to this call.";
  if (escalation === "lastAdmin") return "You are the last admin.";
  if (escalation === "soleMember") return "You are the only member.";
  if (escalation === "startsMekBallot") return "This starts a new channel key for every member.";
  if (escalation === "visibleBeforeUnlock") {
    return "This skin will be visible on this device without your passphrase, before you unlock.";
  }
  if ("attachment" in escalation) {
    const a = escalation.attachment;
    const meta = a.metadataRemoved ? "" : " Its metadata was not removed.";
    return `${a.fileName} (${a.sizeBytes} bytes) is sent.${meta}`;
  }
  if ("containerNotEmpty" in escalation) {
    const c = escalation.containerNotEmpty;
    return `It holds ${c.messages} messages and ${c.members} members.`;
  }
  if ("crossAudience" in escalation) {
    const c = escalation.crossAudience;
    return `It moves from ${c.fromMembers} to ${c.toMembers} people.`;
  }
  if ("massMention" in escalation) return `${escalation.massMention.notified} people are notified.`;
  if ("pendingWork" in escalation) {
    const w = escalation.pendingWork;
    const parts = [
      w.unsent > 0 ? `${w.unsent} changes are still to publish` : null,
      w.inCall ? "you are in a call" : null,
      w.uploading ? "an upload is running" : null,
    ].filter((p) => p !== null);
    return `${parts.join(", ")}.`;
  }
  const k = escalation.verifiedKeyChanged;
  const spaced = (digits: string): string => groupDigits(digits).join(" ");
  return `Their verified key changed: the safety number was ${spaced(k.oldSafetyNumber)} and is now ${spaced(k.newSafetyNumber)}.`;
}

export function reversibilityText(reversibility: Reversibility): string {
  if (reversibility === "undoable") return "You can undo this right after.";
  if (reversibility === "needsOthers") return "Undoing this needs someone else.";
  if (reversibility === "never") return "This cannot be undone.";
  if ("neverBecause" in reversibility) return reasonText(reversibility.neverBecause);
  return `${kindPhrase(reversibility.reversibleByInverse)} reverses it.`;
}

export function notifiesText(notifies: Notifies): string | null {
  if (notifies === "nobody") return null;
  if (notifies === "subject") return "They are told.";
  return `${notifies.members} members are told.`;
}
