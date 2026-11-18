//! The inverse of the canonical encoding (plan E3.2 S7c.4a): a reader
//! decodes a checkpoint's state snapshot (`canonical_state ‖
//! canonical_cut`, the `stateRoot` preimage) back into the state and the
//! cut index it resumes the merge from. Each decoder mirrors its encoder in
//! [`super::canon`] and [`super::state`] field for field, in the same
//! order; [`crate::checkpoint::CutSnapshot`] re-encodes what it decoded and
//! keeps it only when the bytes are equal, so a non-canonical encoding
//! (unsorted or repeated rows, a `JSON` leaf spelt differently) is refused.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::Hash;

use rekindle_types::governance::OpHash;
use rekindle_types::id::{CategoryId, ChannelId, EventId, PseudonymKey, RoleId, ThreadId};

use crate::op::OpStamp;
use crate::state::{
    AdmissionDecision, AutoModRuleState, CategoryState, ChannelSegmentRecord, ChannelState,
    CommunityPolicyState, EventState, ExpressionState, GovernanceState, InviteState, MemberState,
    MetadataState, ModReportState, NotificationDefaultState, OnboardingState, OverwriteState,
    PendingMemberState, RoleState, RsvpState, SegmentState, ThreadState, TimeoutState,
    WelcomeScreenState,
};

/// A cursor over canonical bytes.
pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(n)?;
        let out = self.bytes.get(self.at..end)?;
        self.at = end;
        Some(out)
    }

    /// Every byte was read.
    pub(crate) fn is_done(&self) -> bool {
        self.at == self.bytes.len()
    }

    /// A count or a length; each row a count announces takes at least one
    /// byte, so a false count runs out of bytes instead of looping.
    fn len(&mut self) -> Option<usize> {
        usize::try_from(u32::take(self)?).ok()
    }
}

/// A value with one canonical byte encoding, read back.
pub(crate) trait Decanon: Sized {
    fn take(r: &mut Reader<'_>) -> Option<Self>;
}

/// A shared leaf type written as its serde JSON text (`canon::Json`).
fn take_json<T: serde::de::DeserializeOwned>(r: &mut Reader<'_>) -> Option<T> {
    let n = r.len()?;
    serde_json::from_slice(r.take(n)?).ok()
}

impl Decanon for bool {
    fn take(r: &mut Reader<'_>) -> Option<Self> {
        match r.take(1)? {
            [0] => Some(false),
            [1] => Some(true),
            _ => None,
        }
    }
}

impl Decanon for u32 {
    fn take(r: &mut Reader<'_>) -> Option<Self> {
        Some(Self::from_be_bytes(r.take(4)?.try_into().ok()?))
    }
}

impl Decanon for u64 {
    fn take(r: &mut Reader<'_>) -> Option<Self> {
        Some(Self::from_be_bytes(r.take(8)?.try_into().ok()?))
    }
}

impl<const N: usize> Decanon for [u8; N] {
    fn take(r: &mut Reader<'_>) -> Option<Self> {
        r.take(N)?.try_into().ok()
    }
}

impl Decanon for String {
    fn take(r: &mut Reader<'_>) -> Option<Self> {
        let n = r.len()?;
        Self::from_utf8(r.take(n)?.to_vec()).ok()
    }
}

impl<T: Decanon> Decanon for Option<T> {
    fn take(r: &mut Reader<'_>) -> Option<Self> {
        match r.take(1)? {
            [0] => Some(None),
            [1] => Some(Some(T::take(r)?)),
            _ => None,
        }
    }
}

impl<T: Decanon> Decanon for Vec<T> {
    fn take(r: &mut Reader<'_>) -> Option<Self> {
        let n = r.len()?;
        let mut out = Vec::new();
        for _ in 0..n {
            out.push(T::take(r)?);
        }
        Some(out)
    }
}

impl<A: Decanon, B: Decanon> Decanon for (A, B) {
    fn take(r: &mut Reader<'_>) -> Option<Self> {
        Some((A::take(r)?, B::take(r)?))
    }
}

impl<T: Decanon + Eq + Hash> Decanon for HashSet<T> {
    fn take(r: &mut Reader<'_>) -> Option<Self> {
        Some(Vec::<T>::take(r)?.into_iter().collect())
    }
}

impl<T: Decanon + Ord> Decanon for BTreeSet<T> {
    fn take(r: &mut Reader<'_>) -> Option<Self> {
        Some(Vec::<T>::take(r)?.into_iter().collect())
    }
}

impl<K: Decanon + Eq + Hash, V: Decanon> Decanon for HashMap<K, V> {
    fn take(r: &mut Reader<'_>) -> Option<Self> {
        Some(Vec::<(K, V)>::take(r)?.into_iter().collect())
    }
}

impl<K: Decanon + Ord, V: Decanon> Decanon for BTreeMap<K, V> {
    fn take(r: &mut Reader<'_>) -> Option<Self> {
        Some(Vec::<(K, V)>::take(r)?.into_iter().collect())
    }
}

impl Decanon for OpHash {
    fn take(r: &mut Reader<'_>) -> Option<Self> {
        Some(Self(Decanon::take(r)?))
    }
}

impl Decanon for OpStamp {
    fn take(r: &mut Reader<'_>) -> Option<Self> {
        Some(Self {
            lamport: Decanon::take(r)?,
            hash: Decanon::take(r)?,
        })
    }
}

macro_rules! decanon_id {
    ($($t:ty),*) => {$(
        impl Decanon for $t {
            fn take(r: &mut Reader<'_>) -> Option<Self> {
                Some(Self(Decanon::take(r)?))
            }
        }
    )*};
}

decanon_id!(
    PseudonymKey,
    ChannelId,
    RoleId,
    CategoryId,
    ThreadId,
    EventId
);

/// A struct read field by field in its encoder's order (a struct
/// expression evaluates its fields in the order written); `json` marks a
/// field the encoder wrote as `Json`.
macro_rules! decanon_struct {
    ($t:ident { $($f:ident $(: $json:tt)?),* $(,)? }) => {
        impl Decanon for $t {
            fn take(r: &mut Reader<'_>) -> Option<Self> {
                Some(Self { $($f: decanon_struct!(@field r $($json)?)),* })
            }
        }
    };
    (@field $r:ident) => { Decanon::take($r)? };
    (@field $r:ident json) => { take_json($r)? };
}

decanon_struct!(MemberState {
    member_record,
    member_record_owner,
    joined
});
decanon_struct!(RoleState {
    name,
    permissions,
    position,
    color,
    hoist,
    mentionable,
    self_assignable,
    exclusion_group,
    stamp
});
decanon_struct!(ChannelState {
    name,
    channel_type,
    record_key,
    category_id,
    position,
    topic,
    forum_tags,
    slowmode_seconds,
    nsfw,
    parent_voice_channel_id,
    created
});
decanon_struct!(PendingMemberState {
    member_record,
    member_record_owner,
    stamp
});
decanon_struct!(AdmissionDecision { approved, stamp });
decanon_struct!(TimeoutState {
    duration_seconds,
    started_at,
    stamp
});
decanon_struct!(MetadataState {
    name,
    description,
    icon_hash,
    banner_hash,
    stamp
});
decanon_struct!(NotificationDefaultState { level, stamp });
decanon_struct!(CategoryState {
    name,
    position,
    created
});
decanon_struct!(OverwriteState {
    target_type,
    allow,
    deny,
    stamp
});
decanon_struct!(ThreadState {
    parent_channel_id,
    name,
    thread_type,
    record_key,
    invited,
    forum_tag,
    auto_archive_seconds,
    creator,
    created,
    archived,
    unarchived
});
decanon_struct!(EventState {
    name,
    description,
    start_time,
    end_time,
    channel_id,
    cover_image_ref,
    creator_pseudonym,
    recurrence: json,
    location: json,
    status: json,
    stamp
});
decanon_struct!(RsvpState {
    status: json,
    stamp
});
decanon_struct!(ExpressionState {
    name,
    kind,
    content_hash,
    attachment: json,
    animated,
    tags,
    sound_meta: json,
    creator_pseudonym,
    created_at,
    available_to_peers,
    stamp
});
decanon_struct!(AutoModRuleState {
    name,
    enabled,
    trigger_json,
    action: json,
    stamp
});
decanon_struct!(OnboardingState {
    enabled,
    mode,
    default_channels,
    questions: json,
    welcome_message,
    guide_steps: json,
    stamp
});
decanon_struct!(WelcomeScreenState {
    description,
    channels: json,
    stamp
});
decanon_struct!(SegmentState {
    segment_index,
    registry_key,
    governance_key,
    slot_range_start,
    slot_range_end
});
decanon_struct!(ChannelSegmentRecord { record_key, linked });
decanon_struct!(InviteState {
    code_hash,
    max_uses,
    expires_at,
    secrets_record_key,
    created,
    creator_pseudonym,
    uses
});
decanon_struct!(CommunityPolicyState {
    policy_text,
    max_joins_per_interval,
    join_interval_seconds,
    stamp
});
decanon_struct!(ModReportState {
    reporter,
    target: json,
    reason,
    stamp
});
// `canonical_state`'s section order.
decanon_struct!(GovernanceState {
    members,
    roles,
    role_assignments,
    channels,
    bans,
    op_log_records,
    departed_heads,
    removed,
    removed_by,
    removal_reasons,
    pending_members,
    admitted,
    admission_mode: json,
    timeouts,
    mek_generation,
    metadata,
    notification_default,
    categories,
    overwrites,
    threads,
    events,
    event_rsvps,
    expressions,
    admin_deletes,
    pinned_attachments,
    attachment_pin_stamps,
    creator,
    automod_rules,
    onboarding,
    welcome_screen,
    ban_stamps,
    segments,
    channel_segment_records,
    invites,
    expression_remove_stamps,
    community_policy,
    mod_reports
});
