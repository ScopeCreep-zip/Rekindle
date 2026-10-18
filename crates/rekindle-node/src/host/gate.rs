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
//! lock, against the same attempt limiter the unlock counts on.

use std::sync::Arc;

use rekindle_idempotency::intent_gate::{Committed, Gate, GateError, GateOwner, IntentGate};
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_types::intent::{
    effective_class, kind_of, Classified, IntentId, IntentProof, ProofRequired, Reversibility,
};

use super::api::{Caller, ConnId};
use crate::daemon::dispatch::DaemonContext;

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
        IpcRequest::IntentCancel { intent_id } => {
            if ctx
                .intents
                .cancel(intent_id, &provisional_owner(ctx, caller))
            {
                IpcResponse::ok(&serde_json::json!({ "cancelled": intent_id }))
            } else {
                gate_error(&GateError::UnknownIntent)
            }
        }
        other => begin(ctx, caller, other).await,
    }
}

async fn begin(ctx: &DaemonContext, caller: &Caller, request: IpcRequest) -> IpcResponse {
    let kind = match kind_of(request.name()) {
        Some(Classified::NotAnIntent) => return ctx.dispatch_in_lane(request, caller).await,
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
    let owner = owner_of(ctx, caller, &request);
    match ctx
        .intents
        .begin(owner, selection, request, rekindle_utils::timestamp_ms())
    {
        Ok(Gate::Run(request)) => {
            with_reversibility(ctx.dispatch_in_lane(request, caller).await, reversibility)
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
    let now_ms = rekindle_utils::timestamp_ms();
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
    let ran = ctx
        .intents
        .commit_and_run(
            intent_id,
            &owner,
            &proof,
            fresh,
            move |_| passphrase_ok,
            now_ms,
            |request| async move {
                with_reversibility(ctx.dispatch_in_lane(request, caller).await, reversibility)
            },
        )
        .await;
    match ran {
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
    let checked = tokio::task::spawn_blocking(move || {
        candidates.iter().any(|candidate| {
            super::identity::passphrase_matches(&root, candidate, &passphrase).unwrap_or(false)
        })
    })
    .await;
    checked.unwrap_or(false)
}

fn with_reversibility(response: IpcResponse, reversibility: Reversibility) -> IpcResponse {
    match response {
        IpcResponse::Ok { data, .. } => IpcResponse::Ok {
            data,
            reversibility: Some(reversibility),
        },
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

/// Drop every action held by connection `conn` (it closed).
pub(crate) fn clear_connection(gate: &Arc<HostGate>, conn: u64) {
    gate.clear_matching(|owner| owner.conn == Some(ConnId::Bus(conn)));
}

#[cfg(test)]
#[path = "gate_tests.rs"]
mod tests;
