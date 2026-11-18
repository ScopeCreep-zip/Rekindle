//! A signed test community (plan E3.2): real keys and `build_op`, so its
//! ops verify as any reader verifies them; shared by the join and hinted
//! walk tests.

use crate::join_deps::join_deps;
use crate::op_log::{
    author_record, build_op, frontier_refs, held, verify_held, Deps, HeldOp, OpFate, VerifiedSet,
};
use crate::origin::genesis_entries;
use crate::test_member::Member;
use rekindle_governance::checkpoint::Replay;
use rekindle_governance::merge;
use rekindle_governance::op::{invite_proof_bytes, VerifiedOp};
use rekindle_governance::state::GovernanceState;
use rekindle_secrets::ed25519_dalek::Signer;
use rekindle_secrets::member_record::sign_member_record_binding;
use rekindle_types::governance::{
    AdmissionMode, GenesisCert, GovernanceEntry, InviteProof, OpHash,
};
use rekindle_types::id::ChannelId;

pub(crate) const ANCHOR: &str = "VLD0:anchor";

impl Member {
    pub(crate) fn joined(&self, code: &str) -> GovernanceEntry {
        let record = self.record();
        let sig = self
            .pseudonym
            .sign(&invite_proof_bytes(ANCHOR, code, &record))
            .to_bytes()
            .to_vec();
        let owner_proof =
            sign_member_record_binding(&self.owner.to_bytes(), ANCHOR, &self.author().0, &record);
        GovernanceEntry::MemberJoined {
            member_record: record,
            member_record_owner: self.owner_text(),
            invite_proof: Some(InviteProof {
                code_hash: code.into(),
                sig,
            }),
            owner_proof: owner_proof.to_vec(),
        }
    }
}

pub(crate) fn invite(id: u8, code: &str) -> GovernanceEntry {
    GovernanceEntry::InviteCreated {
        invite_id: [id; 16],
        code_hash: code.into(),
        max_uses: 100,
        expires_at: None,
        secrets_record_key: "VLD0:s".into(),
    }
}

/// A community's held ops, re-verified and re-merged after each op.
pub(crate) struct World {
    pub(crate) cert: GenesisCert,
    pub(crate) held: Vec<HeldOp>,
    pub(crate) set: VerifiedSet,
    pub(crate) state: GovernanceState,
    pub(crate) clock: u64,
}

impl World {
    /// The creator's five genesis ops, then its invite `code`.
    pub(crate) fn new(creator: &Member) -> Self {
        let cert = GenesisCert {
            name: "c".into(),
            policy: AdmissionMode::Open,
            initial_roles: Vec::new(),
            creator_pseudonym: creator.author(),
            creator_member_record: creator.record(),
            creator_member_record_owner: creator.owner_text(),
            created_at: 1,
        };
        let mut world = Self {
            cert,
            held: Vec::new(),
            set: VerifiedSet::default(),
            state: GovernanceState::default(),
            clock: 1,
        };
        let genesis = genesis_entries("c", &creator.author(), ChannelId([2; 16]), "VLD0:ch");
        for body in genesis {
            world.author(creator, body, Deps::Frontier);
        }
        world.author(creator, invite(1, "code"), Deps::Frontier);
        world
    }

    /// `member` authors `body` with `deps` after everything held.
    pub(crate) fn author(
        &mut self,
        member: &Member,
        body: GovernanceEntry,
        deps: Deps,
    ) -> VerifiedOp {
        let op = self.build(member, body, deps);
        self.hold(&op.0, op.1);
        op.0
    }

    pub(crate) fn build(
        &mut self,
        member: &Member,
        body: GovernanceEntry,
        deps: Deps,
    ) -> (VerifiedOp, Vec<u8>) {
        self.clock += 1;
        let (cert, state, set) = (&self.cert, &self.state, &self.set);
        build_op(
            ANCHOR,
            cert,
            state,
            set,
            &member.pseudonym,
            body,
            deps,
            self.clock,
        )
        .unwrap()
    }

    pub(crate) fn hold(&mut self, op: &VerifiedOp, encoded: Vec<u8>) {
        let record = match op.body() {
            GovernanceEntry::MemberJoined { member_record, .. } => member_record.clone(),
            _ => author_record(&self.cert, &self.state, op.author()).unwrap(),
        };
        self.held.push(held(op, record, encoded, OpFate::Applied));
        self.set = verify_held(ANCHOR, &self.held);
        assert!(self.set.pending.is_empty() && self.set.quarantined.is_empty());
        self.state = merge::merge(&self.cert, &self.set.ops);
    }

    /// `member`'s join on `inviter`'s invite `code`, with shallow deps.
    pub(crate) fn join(&mut self, member: &Member, inviter: &Member, code: &str) -> VerifiedOp {
        let deps = self.deps_of(member, inviter);
        self.author(member, member.joined(code), Deps::Join(deps))
    }

    /// `member`'s join deps on `inviter`'s invite, from the held set.
    pub(crate) fn deps_of(&self, member: &Member, inviter: &Member) -> Vec<OpHash> {
        let outcome = merge::merge_outcome(&self.cert, &self.set.ops);
        join_deps(
            &self.cert,
            &self.set,
            &outcome,
            outcome.hints.latest().map(|h| &h.digest),
            &member.author(),
            &inviter.author(),
        )
        .unwrap()
    }

    /// The height-1 checkpoint body over the current frontier.
    pub(crate) fn checkpoint_body(&self) -> rekindle_types::governance::Checkpoint {
        let heads = frontier_refs(&self.cert, &self.state, &self.set);
        let replay = Replay {
            cert: &self.cert,
            ops: &self.set.ops,
            community_id: [7; 32],
        };
        replay.body(None, &heads).expect("a checkpoint body")
    }

    /// `member`'s `Checkpoint` op carrying `body`, citing its cut's heads.
    pub(crate) fn endorse(
        &mut self,
        member: &Member,
        body: rekindle_types::governance::Checkpoint,
    ) -> VerifiedOp {
        let cited = body.cut.iter().map(|r| r.op_hash).collect();
        self.author(
            member,
            GovernanceEntry::Checkpoint(body),
            Deps::Cited(cited),
        )
    }

    /// The hashes of `author`'s op at `seq`.
    pub(crate) fn op_at(&self, author: &Member, seq: u64) -> OpHash {
        let key = author.author();
        self.set
            .ops
            .iter()
            .find(|op| *op.author() == key && op.op().seq == seq)
            .unwrap()
            .hash()
    }
}
