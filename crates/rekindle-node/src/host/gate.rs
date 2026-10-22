//! The one intent gate (plan D3.3, U1.5; ADR 0019; r9 §7.6), in
//! `HostApi::call`: after clearance and `validate_request`, before dispatch.
//!
//! A request the command table does not know is refused (fail closed); a
//! read or plumbing request dispatches; an intent is selected
//! (`intent_select`) and handed to the gate, which runs a Routine one at
//! once (answered with its reversibility) and holds any other, answered
//! with `IntentRequired(IntentReview)`. `IntentCommit` recomputes the
//! selection from the held request and runs it once on the proof its class
//! demands; a passphrase is checked off the executor before the gate's
//! lock, against the same attempt limiter the unlock counts on. A commit
//! of a review held on a peer's verified key changing is the user's
//! acknowledgement of the change ([`Acknowledged`], plan E2.3).

use rekindle_idempotency::intent_gate::{Committed, Gate, GateError, GateOwner, IntentGate};
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_types::intent::{
    effective_class, kind_of, Classified, Escalation, IntentId, IntentProof, ProofRequired,
    Reversibility,
};

use crate::daemon::dispatch::DaemonContext;
use rekindle_ipc::{Caller, ConnId};

/// Who holds an action: the connection that selected it (`None` for one
/// the backend began) and the identity it is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntentOwner {
    pub conn: Option<ConnId>,
    pub identity: String,
}

impl GateOwner for IntentOwner {
    type Identity = String;

    fn identity(&self) -> &String {
        &self.identity
    }

    /// The connection that selected it; for a backend-originated action,
    /// any connection of its identity (first commit wins).
    fn may_act_for(&self, held: &Self) -> bool {
        match held.conn {
            Some(conn) => self.conn == Some(conn),
            None => self.identity == held.identity,
        }
    }
}

/// The host's gate over IPC requests.
pub type HostGate = IntentGate<IpcRequest, IntentOwner, IpcResponse>;

/// What committing a held action acknowledged, for its dispatch (plan
/// E2.3). Signal holds a send to a contact whose verified safety number
/// changed until the user confirms it, and the confirmation is the
/// acknowledgement: here the action's own write clears the change
/// (`signal_identities::acknowledging`), so the action and the
/// acknowledgement commit together; `verified` stays cleared until the
/// user marks the new key verified.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Acknowledged {
    /// The review was held on `VerifiedKeyChanged` for the peer the
    /// request names.
    pub key_change: bool,
}

impl Acknowledged {
    /// Nothing: a request that ran without a review, or whose review held
    /// no acknowledgeable fact.
    pub const NOTHING: Self = Self { key_change: false };

    /// What a commit of a review with `escalations` acknowledges.
    #[must_use]
    pub fn of(escalations: &[Escalation]) -> Self {
        Self {
            key_change: escalations
                .iter()
                .any(|e| matches!(e, Escalation::VerifiedKeyChanged { .. })),
        }
    }
}

/// Validate, refuse what the shell's window owns, gate, dispatch.
pub(crate) async fn run(ctx: &DaemonContext, request: IpcRequest, caller: &Caller) -> IpcResponse {
    if let Err(rejection) = crate::validation::validate_request(&request) {
        return rejection;
    }
    if let Some(refused) = crate::daemon::dispatch::shell::refusal(ctx, &request, caller) {
        return refused;
    }
    match request {
        IpcRequest::IntentCommit { intent_id, proof } => {
            commit(ctx, caller, intent_id, proof).await
        }
        IpcRequest::Undo { id } => {
            let caller_owner = provisional_owner(ctx, caller);
            match ctx.undo.take(id, &caller_owner, ctx.clock.now_ms()) {
                // The inverse is a request of its own: selected, classified
                // and run (or held) as if the user had sent it.
                Some(inverse) => Box::pin(begin(ctx, caller, inverse)).await,
                None => IpcResponse::error(404, "nothing to undo: the offer expired or was used"),
            }
        }
        IpcRequest::IntentCancel { intent_id } => {
            match ctx
                .intents
                .cancel(intent_id, &provisional_owner(ctx, caller))
            {
                // A backend hold's Cancel is the user's answer (a session
                // reset declined, plan E2.1): it runs, unreviewed, since
                // the click is the user's act.
                Ok(Some(on_cancel)) => {
                    let answered = ctx
                        .dispatch_in_lane(on_cancel, caller, Acknowledged::NOTHING)
                        .await;
                    if let IpcResponse::Error { message, .. } = &answered {
                        tracing::warn!(error = %message, "a held action's cancel answer failed");
                    }
                    IpcResponse::ok(&serde_json::json!({ "cancelled": intent_id }))
                }
                Ok(None) => IpcResponse::ok(&serde_json::json!({ "cancelled": intent_id })),
                Err(e) => gate_error(&e),
            }
        }
        other => begin(ctx, caller, other).await,
    }
}

async fn begin(ctx: &DaemonContext, caller: &Caller, request: IpcRequest) -> IpcResponse {
    let kind = match kind_of(request.name()) {
        Some(Classified::NotAnIntent) => {
            return ctx
                .dispatch_in_lane(request, caller, Acknowledged::NOTHING)
                .await
        }
        Some(Classified::Intent(kind)) => kind,
        // `cargo xtask check-intent-table` keeps every request in the
        // table; a name it does not know is refused, not run.
        None => {
            return IpcResponse::error(
                400,
                format!("{} is not in the command table", request.name()),
            )
        }
    };
    let selection = match super::intent_select::select(ctx, &request, kind).await {
        Ok(selection) => selection,
        Err(refused) => return refused,
    };
    let reversibility = selection.reversibility;
    let inverse = super::intent_select::inverse(ctx, &request, reversibility);
    let owner = owner_of(ctx, caller, &request);
    match ctx
        .intents
        .begin(owner.clone(), selection, request, ctx.clock.now_ms())
    {
        // An action that ran at once was never reviewed, so it
        // acknowledges nothing (a `VerifiedKeyChanged` would have held it).
        Ok(Gate::Run(request)) => {
            let response = ctx
                .dispatch_in_lane(request, caller, Acknowledged::NOTHING)
                .await;
            ran(ctx, response, reversibility, owner, inverse)
        }
        Ok(Gate::Review(review)) => IpcResponse::IntentRequired(review),
        Err(e) => gate_error(&e),
    }
}

async fn commit(
    ctx: &DaemonContext,
    caller: &Caller,
    intent_id: IntentId,
    proof: IntentProof,
) -> IpcResponse {
    let provisional = provisional_owner(ctx, caller);
    let Some(held) = ctx.intents.held_action(intent_id, &provisional) else {
        // A repeated commit: the first run consumed the held action.
        return match ctx.intents.ran(intent_id, &provisional).await {
            Some(response) => response,
            None => gate_error(&GateError::UnknownIntent),
        };
    };
    let Some(Classified::Intent(kind)) = kind_of(held.name()) else {
        return gate_error(&GateError::UnknownIntent);
    };
    let fresh = match super::intent_select::select(ctx, &held, kind).await {
        Ok(selection) => selection,
        Err(refused) => return refused,
    };
    let owner = owner_of(ctx, caller, &held);
    let now_ms = ctx.clock.now_ms();
    // Argon2id runs here, off the executor and outside the gate's lock;
    // the gate then counts a failure on the shared limiter.
    let needs_passphrase =
        effective_class(kind, &fresh.escalations).proof() == ProofRequired::Passphrase;
    let passphrase_ok = match (&proof, needs_passphrase) {
        (IntentProof::Passphrase(passphrase), true)
            if ctx.identity_attempts.check(&owner.identity, now_ms).is_ok() =>
        {
            passphrase_opens(ctx, &held, &owner.identity, passphrase).await
        }
        _ => false,
    };
    let reversibility = fresh.reversibility;
    let inverse = super::intent_select::inverse(ctx, &held, reversibility);
    let undo_owner = owner.clone();
    // The commit runs only on the fresh selection matching the review the
    // user answered, so what it acknowledges is what they saw.
    let acknowledged = Acknowledged::of(&fresh.escalations);
    let outcome = ctx
        .intents
        .commit_and_run(
            intent_id,
            &owner,
            &proof,
            fresh,
            move |_| passphrase_ok,
            now_ms,
            |request| async move {
                let response = ctx.dispatch_in_lane(request, caller, acknowledged).await;
                ran(ctx, response, reversibility, undo_owner, inverse)
            },
        )
        .await;
    match outcome {
        Ok(Committed::Ran(response)) => response,
        Ok(Committed::Review(review)) => IpcResponse::IntentRequired(review),
        Err(e) => gate_error(&e),
    }
}

/// The identity an action is for: the one it names, else the unlocked one
/// (none while locked).
fn identity_of(ctx: &DaemonContext, request: &IpcRequest) -> String {
    match request {
        IpcRequest::Unlock { identity, .. } | IpcRequest::IdentityDelete { identity } => {
            identity.clone()
        }
        _ => ctx.require_owner().unwrap_or_default(),
    }
}

fn owner_of(ctx: &DaemonContext, caller: &Caller, request: &IpcRequest) -> IntentOwner {
    IntentOwner {
        conn: Some(caller.conn),
        identity: identity_of(ctx, request),
    }
}

/// The caller before the held request is known: its connection decides
/// (`may_act_for`), the identity is the unlocked one.
fn provisional_owner(ctx: &DaemonContext, caller: &Caller) -> IntentOwner {
    IntentOwner {
        conn: Some(caller.conn),
        identity: ctx.require_owner().unwrap_or_default(),
    }
}

/// Whether `passphrase` opens the vault the action is for: the identity it
/// names or the unlocked one; for a wipe while locked, any identity on the
/// root (every identity it deletes is protected by its own passphrase).
async fn passphrase_opens(
    ctx: &DaemonContext,
    request: &IpcRequest,
    identity: &str,
    passphrase: &str,
) -> bool {
    let root = ctx.root.clone();
    let candidates: Vec<String> =
        if identity.is_empty() && matches!(request, IpcRequest::IdentityWipe) {
            match super::identity::list(&root) {
                Ok(entries) => entries.into_iter().map(|e| e.public_key).collect(),
                Err(e) => {
                    tracing::warn!(error = %e, "identity registry unreadable for a wipe proof");
                    return false;
                }
            }
        } else {
            vec![identity.to_string()]
        };
    let passphrase = zeroize::Zeroizing::new(passphrase.to_string());
    let checked = rekindle_lifecycle::run_blocking(move || {
        candidates.iter().any(|candidate| {
            super::identity::passphrase_matches(&root, candidate, &passphrase).unwrap_or(false)
        })
    })
    .await;
    checked.unwrap_or(false)
}

/// An action that ran: answered with its reversibility and, when it is
/// undoable and its inverse was computed, the undo offer for it.
fn ran(
    ctx: &DaemonContext,
    response: IpcResponse,
    reversibility: Reversibility,
    owner: IntentOwner,
    inverse: Option<IpcRequest>,
) -> IpcResponse {
    match response {
        IpcResponse::Ok { data, .. } => {
            let undo = inverse
                .filter(|_| reversibility == Reversibility::Undoable)
                .and_then(|inverse| {
                    let Some(Classified::Intent(kind)) = kind_of(inverse.name()) else {
                        return None;
                    };
                    Some(ctx.undo.offer(owner, inverse, kind, ctx.clock.now_ms()))
                });
            IpcResponse::Ok {
                data,
                reversibility: Some(reversibility),
                undo,
            }
        }
        other => other,
    }
}

fn gate_error(e: &GateError) -> IpcResponse {
    let code = match e {
        GateError::UnknownIntent => 404,
        GateError::WrongProof => 403,
        GateError::Locked { .. } | GateError::TooManyPending => 429,
    };
    IpcResponse::error(code, e.to_string())
}

/// Drop every action held and every undo offered for connection `conn`
/// (it closed).
pub(crate) fn clear_connection(ctx: &DaemonContext, conn: u64) {
    let closed = |owner: &IntentOwner| owner.conn == Some(ConnId::Bus(conn));
    ctx.intents.clear_matching(closed);
    ctx.undo.clear_matching(closed);
}

#[cfg(test)]
#[path = "gate_tests.rs"]
mod tests;
