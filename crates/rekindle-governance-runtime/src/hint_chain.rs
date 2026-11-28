//! The hints a joiner may start from (plan E3.2 S7c.5, "Using a
//! checkpoint"): before its walk it asks its inviter, by the invite it
//! holds, for the hint selected at the greatest height
//! (`checkpointHintRequest`), then height by height for the hint each
//! `prevCheckpoint` names, down to height 1. Up from height 1 it checks
//! each height's endorsements against the eligible set the height below
//! leaves (`endorsed_link`, chained to the `GenesisCert`), its directory
//! pages against `dirRoot` (`pages_match`), and fetches its snapshot from
//! the inviter and checks it against `stateRoot` (`fetch_state_snapshot`,
//! `CutSnapshot::decode`); that snapshot's state is the electorate of the
//! height above. The verified hints are returned newest first; the first
//! failed height ends the chain, so the heights below it stay usable. A
//! community holding a checkpoint equivocation answers no hint (S7c.4).

use std::collections::BTreeSet;

use futures::stream::{self, StreamExt};
use rekindle_codec::governance_op::decode_op;
use rekindle_governance::checkpoint::directory::DirectoryPage;
use rekindle_governance::checkpoint::{
    checkpoint_digest, endorsed_link, pages_match, CutSnapshot, EndorsedLink, SignedEndorsement,
};
use rekindle_records::port::OpenIntent;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::governance::{GenesisCert, OpRef};
use rekindle_types::key_format::vld0_key_bytes;

use crate::checkpoint_hint::request_hint;
use crate::checkpoint_state::fetch_state_snapshot;
use crate::deps::GovernanceRuntimeDeps;
use crate::op_log::fetch_held;
use crate::op_walk::{WalkLogCache, WALK_FETCHES_IN_FLIGHT};

/// A hint whose endorsement chain, pages and snapshot verified.
#[derive(Debug, Clone)]
pub struct VerifiedHint {
    pub link: EndorsedLink,
    pub snapshot: CutSnapshot,
}

/// Who asks, and through whom: the joiner's pseudonym in the community,
/// the inviter's route and the invite code, which tags each request (plan
/// E3.2 S7c.8 (2)).
pub struct Asker<'a> {
    pub me: &'a SigningKey,
    pub route: &'a [u8],
    pub code: &'a str,
}

/// The endorsements `refs` name, fetched and checked by signature.
async fn endorsements<D: GovernanceRuntimeDeps>(
    deps: &D,
    anchor: &str,
    refs: &[OpRef],
    logs: &WalkLogCache,
) -> Option<Vec<SignedEndorsement>> {
    let mut out = Vec::with_capacity(refs.len());
    for r in refs {
        let held = fetch_held(deps, r, logs).await.ok()??;
        let decoded = decode_op(&held.encoded).ok()?;
        out.push(SignedEndorsement::verify(
            anchor,
            &decoded.op,
            &decoded.unsigned,
        )?);
    }
    Some(out)
}

/// Each height's endorsements, from the selected hint down to height 1,
/// newest first. The endorsements one height up name their
/// `prevCheckpoint`s, which are fetched and counted with that height's;
/// a level whose hint is not the one asked for ends the walk down.
async fn levels<D: GovernanceRuntimeDeps>(
    deps: &D,
    anchor: &str,
    asker: &Asker<'_>,
) -> Vec<Vec<SignedEndorsement>> {
    let logs = WalkLogCache::default();
    let mut out: Vec<Vec<SignedEndorsement>> = Vec::new();
    let mut ask = (0u64, [0u8; 32]);
    let mut named: Vec<SignedEndorsement> = Vec::new();
    loop {
        let response =
            match request_hint(deps, asker.route, anchor, asker.me, ask, Some(asker.code)).await {
                Ok(response) => response,
                Err(e) => {
                    tracing::debug!(error = %e, "no checkpoint hint from the inviter");
                    break;
                }
            };
        let Some(mut level) = endorsements(deps, anchor, &response.endorsements, &logs).await
        else {
            break;
        };
        let Some(first) = level.first().map(|e| e.body.clone()) else {
            break;
        };
        if ask.0 != 0 && (first.height != ask.0 || checkpoint_digest(&first) != ask.1) {
            break;
        }
        let have: BTreeSet<_> = level.iter().map(|e| e.hash).collect();
        level.extend(named.into_iter().filter(|e| !have.contains(&e.hash)));
        let prevs: Vec<OpRef> = {
            let mut seen = BTreeSet::new();
            (level.iter())
                .filter_map(|e| e.body.prev.clone())
                .filter(|p| seen.insert(p.op_hash))
                .collect()
        };
        out.push(level);
        if prevs.is_empty() {
            break;
        }
        let Some(below) = endorsements(deps, anchor, &prevs, &logs).await else {
            break;
        };
        let Some(next) = below.first().map(|e| e.body.clone()) else {
            break;
        };
        ask = (next.height, checkpoint_digest(&next));
        named = below;
    }
    out
}

/// The directory pages `keys` name, read in parallel (at most
/// [`WALK_FETCHES_IN_FLIGHT`]), in page order; `None` when one is missing
/// or does not decode.
async fn pages<D: GovernanceRuntimeDeps>(deps: &D, keys: &[String]) -> Option<Vec<DirectoryPage>> {
    let read = |key: String| async move {
        let lease = deps
            .acquire_record(&key, None, OpenIntent::Check)
            .await
            .ok()?;
        let value = deps.get_dht_value(lease, 0, true).await;
        deps.release_record(lease).await;
        DirectoryPage::decode(&value.ok()??).ok()
    };
    let found: Vec<Option<DirectoryPage>> = stream::iter(keys.iter().cloned().map(read))
        .buffered(WALK_FETCHES_IN_FLIGHT)
        .collect()
        .await;
    found.into_iter().collect()
}

/// The hints a joiner may start from in community `anchor` under `cert`,
/// asked of its inviter as `asker`: verified, newest first; empty when
/// the inviter answers none or height 1 does not verify.
pub async fn verified_hints<D: GovernanceRuntimeDeps>(
    deps: &D,
    anchor: &str,
    cert: &GenesisCert,
    asker: &Asker<'_>,
) -> Vec<VerifiedHint> {
    let Some(community) = vld0_key_bytes(anchor) else {
        return Vec::new();
    };
    let mut chain: Vec<VerifiedHint> = Vec::new();
    for level in levels(deps, anchor, asker).await.iter().rev() {
        let below = chain.last().map(|h| (&h.link, &h.snapshot.state));
        let Some(link) = endorsed_link(cert, anchor, below, level) else {
            break;
        };
        let pages_hold = match pages(deps, &link.body.pages).await {
            Some(pages) => pages_match(&community, &link.body, &pages),
            None => false,
        };
        if !pages_hold {
            tracing::warn!(height = link.height, "checkpoint pages miss their dirRoot");
            break;
        }
        let route = [asker.route.to_vec()];
        let fetched =
            fetch_state_snapshot(deps, anchor, asker.me, &link.body, &route, Some(asker.code))
                .await;
        let Some(snapshot) = fetched
            .ok()
            .flatten()
            .and_then(|bytes| CutSnapshot::decode(&link.body, &bytes))
        else {
            break;
        };
        chain.push(VerifiedHint { link, snapshot });
    }
    chain.reverse();
    chain
}
