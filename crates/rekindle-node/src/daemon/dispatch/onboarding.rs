//! A community's onboarding and welcome-screen requests (plan E3.1 D3),
//! answered by the Host's governance adapter on both hosts.

use rekindle_ipc::protocol::IpcResponse;
use rekindle_types::dto::onboarding::{OnboardingConfigDto, WelcomeScreenDto};
use rekindle_types::presence::OnboardingAnswer;

use super::governance::writable;
use super::{adapter, state_error, DaemonContext};
use crate::daemon::DaemonState;
use crate::host::adapters::governance::onboarding::OnboardingRefusal;

fn answered(result: Result<(), OnboardingRefusal>) -> IpcResponse {
    match result {
        Ok(()) => IpcResponse::ok(&()),
        Err(r) => IpcResponse::error(u32::from(r.code()), r.to_string()),
    }
}

/// The community a read is about, once we may query.
fn readable(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
) -> Result<String, IpcResponse> {
    if !state.can_query() {
        return Err(state_error(state, "query"));
    }
    Ok(ctx.resolve_community(community)?.governance_key)
}

/// The merged onboarding flow.
pub(crate) fn handle_config_get(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
) -> IpcResponse {
    match readable(ctx, state, community) {
        Ok(key) => IpcResponse::ok(&adapter(ctx).onboarding_config(&key)),
        Err(e) => e,
    }
}

/// The merged welcome screen.
pub(crate) fn handle_welcome_get(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
) -> IpcResponse {
    match readable(ctx, state, community) {
        Ok(key) => IpcResponse::ok(&adapter(ctx).welcome_screen(&key)),
        Err(e) => e,
    }
}

/// Write the onboarding flow.
pub(crate) async fn handle_config_set(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    config: &OnboardingConfigDto,
) -> IpcResponse {
    match writable(ctx, state, community) {
        Ok(key) => answered(adapter(ctx).set_onboarding_config(&key, config).await),
        Err(e) => e,
    }
}

/// Write the welcome screen.
pub(crate) async fn handle_welcome_set(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    screen: &WelcomeScreenDto,
) -> IpcResponse {
    match writable(ctx, state, community) {
        Ok(key) => answered(adapter(ctx).set_welcome_screen(&key, screen).await),
        Err(e) => e,
    }
}

/// Answer the onboarding questions.
pub(crate) async fn handle_submit(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    answers: Vec<OnboardingAnswer>,
    acknowledged_rules: bool,
) -> IpcResponse {
    match writable(ctx, state, community) {
        Ok(key) => answered(
            adapter(ctx)
                .submit_onboarding_answers(&key, answers, acknowledged_rules)
                .await,
        ),
        Err(e) => e,
    }
}

/// Our onboarding is done.
pub(crate) async fn handle_complete(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    match ctx.resolve_community(community) {
        Ok(m) => answered(
            adapter(ctx)
                .mark_onboarding_complete(&m.governance_key)
                .await,
        ),
        Err(e) => e,
    }
}
