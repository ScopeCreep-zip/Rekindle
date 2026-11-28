# 0019 — Intentional actions: one backend gate classifies every user action

- **Status:** Accepted (owner ruling O6, 2026-10-15: "intents" means Nintendo-style three-step intentional
  actions)
- **Date:** 2026-10-15
- **Plan step:** U1 (after V, before C9) in `.claude/plans/standards-remediation/00-integration-plan.md` (v2.2).
  Attachment metadata stripping goes with `rekindle-files` at E5.1.
- **Research:** `.claude/plans/standards-remediation/realignment/r9-intents.md`. Sources are saved in
  `.claude/plans/standards-remediation/evidence/sources/realignment/intents/`.
- **Related:** [0010](0010-single-daemon-thin-frontends.md) (the Host owns policy; GUI, TUI and CLI are thin).

## Context and problem statement

The owner asked for Nintendo's "3-step intentional actions for any flow, such as sending a message or
purchasing" (`.claude/plans/architecture-audit/history/h3-owner-voice.md:79`). No step implemented it. Today:

- block, remove friend, unblock, unban and logout have no confirmation in the GUI;
- the same action confirms in one frontend and not another (remove friend and leave voice confirm in the TUI,
  `crates/rekindle-tui/src/app/reducer.rs:309-340`; channel delete, role delete, transfer and identity rotate
  confirm only in the CLI);
- the daemon checks proof only for `IdentityDestroy`/`IdentityWipe` (`crates/rekindle-node/src/validation.rs`),
  with fixed phrases.

The memory note that mapped "intentional action" to the W16 delivery queue is superseded by this ruling.

## Evidence

- **Nintendo's flows are Select → Review → Commit,** with a PIN or password before commit for protected assets
  (`nintendo-eshop-purchase`, `nintendo-switch2-password-pin-types`). Messaging puts the steps on the
  relationship: "both sides must manually approve each other" (`nintendo-gamechat-faq-uk.txt:243`).
- **Extra clicks don't change decisions:** Akhawe & Felt, USENIX Security 2013, 25 M warning impressions
  (`akhawe-felt-2013-alice-in-warningland.pdf`).
- **Typing beats clicking:** 74 % for a typed confirmation against ≤ 20 % for extra clicks (Bravo-Lillo et al.,
  SOUPS 2013, `bravo-lillo-2013-your-attention-please.pdf`). People habituate to repeated dialogs (Vance et al.,
  CHI 2017).
- **Platform guidance:** Apple HIG "Avoid displaying alerts for common, undoable actions"; WCAG 2.2 SC 3.3.4
  "not the intent to require a confirmation for each save"; GitHub's typed repository delete.
- **Signal** confirms a send only after a verified contact's safety number changes (`signal-zd-360007060632`).

## Decision outcome

**One intent table and one gate, owned by the backend.**

- **Classes** (`rekindle-types::intent`, Tier 1, pure data), each with its step 3 proof:

  | Class | Use | Proof | CLI without a terminal |
  |---|---|---|---|
  | Routine | frequent, undoable, or abandoning a task | none (undo toast where an inverse exists) | runs |
  | Consequential | others see or are affected; self-protective acts | a button named verb + object, no default | `--yes` |
  | TrustChanging | raises exposure or changes key trust (admit a person, unblock, safety-number reset/accept, pairing, first screen share in a call) | tick a box that states the fact (fingerprint, what becomes visible) | `--confirm=<name>` |
  | Destructive | irreversible loss of shared history or for others | type the object's name | `--confirm=<name>` |
  | Protected | root identity or key material | passphrase, every time | passphrase reader only |

  The table is a `match` with no wildcard, so a new action does not compile until it is classified. Protective
  acts (block, decline, revoke) are capped at Consequential, so a survivor can always block in three quick moves.
- **Backend escalations** raise a class only for real risk, computed from live state in the owning tier crate:
  a verified key changed (send held until acknowledged), a mass mention, an attachment (the preview is the
  review), a forward into another audience, pending work on logout, the last admin leaving, deleting a
  container with history.
- **One gate in the Host** (`rekindle-node::intents`): a non-Routine request returns
  `IpcResponse::IntentRequired(IntentReview)`; `IntentCommit`/`IntentCancel` finish it; backend-originated
  intents (deep links, incoming session reset, pairing) arrive as `SubscriptionEvent::IntentPending`. Pending
  intents are bound to connection and identity, expire, and are cleared on lock, logout and identity switch.
  `IntentId` is the idempotency key. Routine requests skip the gate, so send and typing add no round trip.
- **Message send** stays one step. The three steps sit on admitting the relationship, plus the escalations above.
- **Frontends render only.** GUI `IntentDialog`, TUI `IntentOverlay`, CLI prompts. `--force` is renamed `--yes`,
  and `--yes` never satisfies TrustChanging, Destructive or Protected.
- **Enforcement:** `cargo xtask check-intent-table` fails the build when a registered `#[tauri::command]` or an
  `IpcRequest` variant is missing from the table.

## Consequences

- `ConfirmDialog`, the `window.confirm` session-reset call, the TUI's local confirm arming and the CLI's
  `confirm_unless_forced`/`confirm_severe` decisions are deleted or reduced to rendering.
- The daemon's fixed phrases `DESTROY MY IDENTITY` and `WIPE ALL DATA` become passphrase proof.
- Leave voice stops confirming in the TUI (Routine).
- Prompt rates per class are budgeted in E2E journeys, counted only in dev and test builds, never sent anywhere.
- The panic wipe (E7.1) is armed as a Protected intent and triggered in one act.
- If the owner later rules that every message needs a review, only the `ChannelSend`/`DmSend` rows change.

## More information

- [WCAG 2.2 SC 3.3.4](https://www.w3.org/WAI/WCAG22/Understanding/error-prevention-legal-financial-data.html),
  [Apple HIG: Alerts](https://developer.apple.com/design/human-interface-guidelines/alerts),
  [clig.dev](https://clig.dev/)
- `.claude/docs/frontend-wiring-guide.md` (the UI spec adopted by plan v2.2)
