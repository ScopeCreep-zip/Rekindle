//! A checkpoint body from one replay (plan E3.2 S7c.4): what a proposer
//! publishes and what an endorser recomputes from its own replay at that
//! cut before it signs (Autobase: never sign past what you processed).

use rekindle_types::governance::{Checkpoint, GenesisCert, GovernanceEntry, OpHash, OpRef};
use rekindle_types::key_format::vld0_key_bytes;

use super::cut::{causal_past, state_root};
use super::directory::{CheckpointDirectory, DirLeafV1, MemberRecordKey};
use super::electorate::{eligible_set, eligible_set_hash};
use super::shape::{checkpoint_digest, frontier_hash, MAX_CUT_HEADS};
use crate::merge::merge_cut;
use crate::op::VerifiedOp;
use crate::state::GovernanceState;

/// One reader's replay inputs: the pinned cert, the verified op set and
/// the community id the directory's leaves carry (the anchor record's
/// opaque key bytes).
#[derive(Clone, Copy)]
pub struct Replay<'a> {
    pub cert: &'a GenesisCert,
    pub ops: &'a [VerifiedOp],
    pub community_id: [u8; 32],
}

impl Replay<'_> {
    /// The `Checkpoint` body of the op `at` names, if held.
    fn checkpoint(&self, at: &OpRef) -> Option<&Checkpoint> {
        self.ops.iter().find_map(|o| match o.body() {
            GovernanceEntry::Checkpoint(c) if o.hash() == at.op_hash => Some(c),
            _ => None,
        })
    }

    fn state_at(&self, heads: &[OpRef]) -> GovernanceState {
        merge_cut(self.cert, &causal_past(self.ops, heads)).0
    }

    /// The directory at `cp`: its previous checkpoint's leaves in their
    /// order, then each member first admitted in its cut, by the
    /// `(lamport, op_hash)` of the join the state holds (append-only along
    /// the `prevCheckpoint` chain, S7c.2). A member the cert binds (the
    /// creator) has no `MemberJoined`: its leaf names its seq-0 op.
    fn directory(&self, cp: Option<&Checkpoint>) -> Option<CheckpointDirectory> {
        let Some(cp) = cp else {
            return Some(CheckpointDirectory::default());
        };
        let prev = match &cp.prev {
            Some(at) => Some(self.checkpoint(at)?),
            None => None,
        };
        let mut dir = self.directory(prev)?;
        let closure = causal_past(self.ops, &cp.cut);
        let state = merge_cut(self.cert, &closure).0;
        let mut fresh: Vec<((u64, OpHash), DirLeafV1)> = Vec::new();
        for (member, held) in &state.members {
            if dir.leaves().iter().any(|l| l.member == *member) {
                continue;
            }
            let join = closure.iter().find(|o| {
                o.author() == member
                    && if held.joined == crate::op::OpStamp::MIN {
                        o.op().seq == 0
                    } else {
                        matches!(o.body(), GovernanceEntry::MemberJoined { .. })
                            && o.stamp() == held.joined
                    }
            })?;
            let leaf = DirLeafV1 {
                community_id: self.community_id,
                member: member.clone(),
                member_record: MemberRecordKey::from_text(&held.member_record)?,
                member_record_owner: vld0_key_bytes(&held.member_record_owner)?,
                join_op_hash: join.hash(),
                join_height: cp.height,
            };
            fresh.push(((join.lamport(), join.hash()), leaf));
        }
        fresh.sort_by_key(|(key, _)| *key);
        for (_, leaf) in fresh {
            dir.push(leaf);
        }
        Some(dir)
    }

    /// The body of the checkpoint over the cut `heads` on top of `prev` (the
    /// op naming the previous checkpoint; `None` at height 1), round 0 and
    /// no pages. `None` when a head or `prev` is not held, the cut does not
    /// contain `prev`'s cut, a cut is out of bounds, or a member's record
    /// keys do not parse.
    #[must_use]
    pub fn body(&self, prev: Option<&OpRef>, heads: &[OpRef]) -> Option<Checkpoint> {
        let held = |r: &OpRef| self.ops.iter().any(|o| o.hash() == r.op_hash);
        if heads.is_empty() || heads.len() > MAX_CUT_HEADS || !heads.iter().all(held) {
            return None;
        }
        let closure = causal_past(self.ops, heads);
        let prev_body = match prev {
            Some(at) => Some(self.checkpoint(at)?),
            None => None,
        };
        let electorate = match prev_body {
            Some(p) => {
                let inside = |r: &OpRef| closure.iter().any(|o| o.hash() == r.op_hash);
                if !p.cut.iter().all(inside) {
                    return None;
                }
                eligible_set(self.cert, Some(&self.state_at(&p.cut)))
            }
            None => eligible_set(self.cert, None),
        };
        let (state, index) = merge_cut(self.cert, &closure);
        let head_hashes: Vec<OpHash> = heads.iter().map(|r| r.op_hash).collect();
        let mut body = Checkpoint {
            height: prev_body.map_or(1, |p| p.height + 1),
            round: 0,
            prev: prev.cloned(),
            cut: heads.to_vec(),
            frontier_hash: frontier_hash(&head_hashes),
            dir_root: [0; 32],
            dir_size: 0,
            state_root: state_root(&state, &index),
            mek_generation: state.mek_generation,
            eligible_set_hash: eligible_set_hash(&electorate),
            pages: Vec::new(),
        };
        let dir = self.directory(Some(&body))?;
        body.dir_root = dir.root().0;
        body.dir_size = dir.size();
        Some(body)
    }

    /// The directory of a body [`Self::body`] built or an endorser
    /// recomputed: what the proposer seals into pages.
    #[must_use]
    pub fn directory_of(&self, body: &Checkpoint) -> Option<CheckpointDirectory> {
        self.directory(Some(body))
    }

    /// Whether our replay at `proposal`'s cut gives the identical body.
    #[must_use]
    pub fn recomputes(&self, proposal: &Checkpoint) -> bool {
        self.body(proposal.prev.as_ref(), &proposal.cut)
            .is_some_and(|ours| checkpoint_digest(&ours) == checkpoint_digest(proposal))
    }
}
