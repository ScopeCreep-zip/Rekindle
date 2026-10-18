/**
 * The words of an intent review (ADR 0019): the action as a verb and its
 * object, the subject, and each typed fact and escalation the Host states.
 * Pure: the Host decides what the review says; this only words it.
 */

import type {
  Escalation, Fact, IntentKind, IntentReview, Notifies, Reversibility, Subject,
} from "../ipc/commands/dto";

/** `deleteChannel` → `Delete channel`. */
export function kindPhrase(kind: IntentKind): string {
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
  if ("person" in subject) return subject.person.localNickname ?? subject.person.fingerprint;
  if ("community" in subject) return subject.community.name;
  if ("channel" in subject) return subject.channel.name;
  if ("role" in subject) return subject.role.name;
  if ("file" in subject) return subject.file.name;
  if ("link" in subject) return subject.link.host;
  if ("ownIdentity" in subject) return subject.ownIdentity.name;
  return subject.named.name;
}

/** The final button: the verb and its object, never "OK". */
export function commitLabel(review: IntentReview): string {
  return `${kindPhrase(review.kind)}: ${subjectLabel(review.subject)}`;
}

export function factText(fact: Fact): string {
  if (fact === "subjectNotTold") return "They are not told.";
  if (fact === "pendingMessagesDropped") return "Messages waiting from them are dropped.";
  if (fact === "sessionDeleted") return "Your secure session with them is deleted.";
  if (fact === "presenceVisible") return "They will see when you are online and can reach you.";
  if (fact === "ipAddressLeaves") return "The site will see your IP address.";
  if (fact === "rejoinNeedsInvite") return "Rejoining needs a new invite.";
  if ("fingerprint" in fact) return `Key fingerprint: ${fact.fingerprint.fingerprint}`;
  if ("safetyNumber" in fact) return `Safety number: ${fact.safetyNumber.safetyNumber}`;
  if ("membersAffected" in fact) return `${fact.membersAffected.count} members are affected.`;
  return `${fact.messagesDeleted.count} messages are deleted for everyone.`;
}

export function escalationText(escalation: Escalation): string {
  if (escalation === "firstMediaInCall") return "This is the first time your camera or screen goes to this call.";
  if (escalation === "lastAdmin") return "You are the last admin.";
  if (escalation === "soleMember") return "You are the only member.";
  if (escalation === "startsMekBallot") return "This starts a new channel key for every member.";
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
  return `Their verified key changed: ${k.oldSafetyNumber} → ${k.newSafetyNumber}.`;
}

export function reversibilityText(reversibility: Reversibility): string {
  if (reversibility === "undoable") return "You can undo this right after.";
  if (reversibility === "needsOthers") return "Undoing this needs someone else.";
  if (reversibility === "never") return "This cannot be undone.";
  return `${kindPhrase(reversibility.reversibleByInverse)} reverses it.`;
}

export function notifiesText(notifies: Notifies): string | null {
  if (notifies === "nobody") return null;
  if (notifies === "subject") return "They are told.";
  return `${notifies.members} members are told.`;
}
