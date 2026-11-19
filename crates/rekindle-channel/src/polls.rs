//! Channel polls (plan E3.3): a create, vote or close is an op on its
//! author's channel outbox (`M`/4), queued on the channel's `channel_post`
//! stream; a vote or close is judged against the poll as our reader
//! aggregates it from the authors' outboxes.

use std::collections::HashMap;

use rekindle_codec::community::channel_record::ChannelPollCounted;
use rekindle_types::domains;
use rekindle_types::id::PseudonymKey;
use rekindle_types::message::{ChannelPost, PostEntry};

use crate::deps::ChannelMessagingDeps;
use crate::error::ChannelError;

/// A poll's state as our reader aggregates it from its authors' outboxes
/// ([`crate::history::poll_tallies`]): what a vote or close is judged by.
pub struct PollSnapshot {
    /// The author's pseudonym hex.
    pub author: String,
    pub answer_count: usize,
    pub multi_select: bool,
    pub expired: bool,
    pub closed: bool,
    /// Each voter's counted answers, by pseudonym hex.
    pub latest_votes: HashMap<String, Vec<u8>>,
    /// Each counted voter's stream seq of the vote counted: what our close
    /// names (plan E3.3, rule E1).
    pub counted: Vec<(String, u64)>,
}

/// Validate poll-create inputs. Pure — no deps.
pub fn validate_poll_create(question: &str, answers: &[String]) -> Result<(), ChannelError> {
    if question.trim().is_empty() {
        return Err(ChannelError::Adapter(
            "poll question cannot be empty".to_string(),
        ));
    }
    if answers.len() < 2 || answers.len() > 10 {
        return Err(ChannelError::Adapter(
            "polls must have between 2 and 10 answers".to_string(),
        ));
    }
    if answers.iter().any(|answer| answer.trim().is_empty()) {
        return Err(ChannelError::Adapter(
            "poll answers cannot be empty".to_string(),
        ));
    }
    Ok(())
}

/// The id of the poll `creator` made with `nonce` (plan E3.3, rule E1): a
/// poll is bound to its creator, so a create by another member, however
/// it is dated, names another poll and can't take this one over.
#[must_use]
pub fn poll_id_of(creator: &PseudonymKey, nonce: &[u8; 16]) -> [u8; 16] {
    let mut input = [0u8; 48];
    input[..32].copy_from_slice(&creator.0);
    input[32..].copy_from_slice(nonce);
    let digest = blake3::derive_key(domains::CHANNEL_POLL_ID, &input);
    let mut id = [0u8; 16];
    id.copy_from_slice(&digest[..16]);
    id
}

pub fn parse_poll_id(poll_id_hex: &str) -> Result<[u8; 16], ChannelError> {
    hex::decode(poll_id_hex)
        .map_err(|e| ChannelError::Adapter(format!("invalid poll id hex: {e}")))?
        .try_into()
        .map_err(|_| ChannelError::Adapter("poll id must be 16 bytes".to_string()))
}

#[must_use]
pub fn dedupe_selected_answers(selected_answers: Vec<u8>) -> Vec<u8> {
    let mut selected_answers = selected_answers;
    selected_answers.sort_unstable();
    selected_answers.dedup();
    selected_answers
}

/// Queue `entry` (a poll op) on `channel_id`'s `channel_post` stream: an op
/// on our channel outbox (`M`/4) with no `messages` row.
async fn queue_poll_op<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    entry: PostEntry,
) -> Result<(), ChannelError> {
    let post = ChannelPost {
        community: community_id.to_owned(),
        channel: channel_id.to_owned(),
        message: format!(
            "poll:{}",
            hex::encode(rekindle_utils::random::id_bytes_16())
        ),
        entry,
    };
    deps.queue_post(&post).await
}

fn require_channel<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
) -> Result<(), ChannelError> {
    deps.channel_info(community_id, channel_id)
        .map(drop)
        .ok_or_else(|| ChannelError::ChannelNotFound(channel_id.into()))
}

pub async fn persist_poll_create<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    message_id: &str,
    question: &str,
    answers: Vec<String>,
    multi_select: bool,
    duration_seconds: Option<u64>,
) -> Result<String, ChannelError> {
    validate_poll_create(question, &answers)?;
    require_channel(deps, community_id, channel_id)?;
    let me = (deps.my_pseudonym_hex(community_id))
        .and_then(|key| hex::decode(key).ok()?.try_into().ok())
        .map(PseudonymKey)
        .ok_or_else(|| ChannelError::Adapter("no pseudonym in this community".to_string()))?;
    let nonce = rekindle_utils::random::id_bytes_16();
    let poll_id = poll_id_of(&me, &nonce);
    let entry = PostEntry::PollCreate {
        nonce,
        target: message_id.to_string(),
        question: question.trim().to_string(),
        answers: answers
            .into_iter()
            .map(|answer| answer.trim().to_string())
            .collect(),
        multi_select,
        expires_at: duration_seconds.map(|seconds| deps.now_secs().saturating_add(seconds)),
    };
    queue_poll_op(deps, community_id, channel_id, entry).await?;
    Ok(hex::encode(poll_id))
}

pub async fn persist_poll_vote<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    poll_id_hex: &str,
    selected_answers: Vec<u8>,
) -> Result<(), ChannelError> {
    if selected_answers.is_empty() {
        return Err(ChannelError::Adapter(
            "at least one answer must be selected".to_string(),
        ));
    }
    require_channel(deps, community_id, channel_id)?;
    let poll_id = parse_poll_id(poll_id_hex)?;
    let snapshot = load_poll_snapshot(deps, community_id, channel_id, poll_id).await?;
    validate_poll_vote(&snapshot, &selected_answers)?;
    let entry = PostEntry::PollVote {
        poll_id,
        selected_answers: dedupe_selected_answers(selected_answers),
    };
    queue_poll_op(deps, community_id, channel_id, entry).await
}

pub async fn persist_poll_close<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    poll_id_hex: &str,
    allow_moderator_override: bool,
) -> Result<(), ChannelError> {
    require_channel(deps, community_id, channel_id)?;
    let poll_id = parse_poll_id(poll_id_hex)?;
    let snapshot = load_poll_snapshot(deps, community_id, channel_id, poll_id).await?;
    if !allow_moderator_override {
        let me = deps.my_pseudonym_hex(community_id).unwrap_or_default();
        if snapshot.author != me {
            return Err(ChannelError::PermissionDenied(
                "only the poll author or a moderator can close this poll".to_string(),
            ));
        }
    }
    // The close names the votes our reader counts now; readers count
    // exactly those, once every part is read.
    for part in poll_close_parts(poll_id, &snapshot.counted)? {
        queue_poll_op(deps, community_id, channel_id, part).await?;
    }
    Ok(())
}

/// The voters one close part names: an entry is at most one sealed page
/// (32,768 B less the page and entry overhead, plan E3.3), and a voter
/// costs at most ~100 B of the part's JSON (a 64-char pseudonym hex and a
/// u64 seq), so 256 voters leave room to spare.
pub const CLOSE_VOTERS_PER_PART: usize = 256;

/// The parts of our close of `poll_id` over `counted` (voter, stream seq),
/// [`CLOSE_VOTERS_PER_PART`] voters each, sharing one fresh close id; a
/// close with no votes is one empty part.
///
/// # Errors
/// More parts than a `u16` counts.
pub fn poll_close_parts(
    poll_id: [u8; 16],
    counted: &[(String, u64)],
) -> Result<Vec<PostEntry>, ChannelError> {
    let chunks: Vec<Vec<(String, u64)>> = if counted.is_empty() {
        vec![Vec::new()]
    } else {
        counted
            .chunks(CLOSE_VOTERS_PER_PART)
            .map(<[_]>::to_vec)
            .collect()
    };
    let parts = u16::try_from(chunks.len())
        .map_err(|_| ChannelError::Adapter("too many voters to close".to_string()))?;
    let close_id = rekindle_utils::random::id_bytes_16();
    Ok((0u16..)
        .zip(chunks)
        .map(|(part, counted)| PostEntry::PollClose {
            poll_id,
            close_id,
            part,
            parts,
            counted,
        })
        .collect())
}

/// `counted` (voter, stream seq) as a close entry names it.
#[must_use]
pub fn counted_entries(counted: &[(String, u64)]) -> Vec<ChannelPollCounted> {
    (counted.iter())
        .map(|(voter, stream_seq)| ChannelPollCounted {
            voter: voter.clone(),
            stream_seq: *stream_seq,
        })
        .collect()
}

pub async fn get_poll_results<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    poll_id_hex: &str,
) -> Result<Vec<u32>, ChannelError> {
    require_channel(deps, community_id, channel_id)?;
    let poll_id = parse_poll_id(poll_id_hex)?;
    let snapshot = load_poll_snapshot(deps, community_id, channel_id, poll_id).await?;
    let mut counts = vec![0_u32; snapshot.answer_count];
    for selected in snapshot.latest_votes.into_values() {
        for index in selected {
            if let Some(count) = counts.get_mut(usize::from(index)) {
                *count = count.saturating_add(1);
            }
        }
    }
    Ok(counts)
}

fn validate_poll_vote(
    snapshot: &PollSnapshot,
    selected_answers: &[u8],
) -> Result<(), ChannelError> {
    if snapshot.closed {
        return Err(ChannelError::Adapter("poll is closed".to_string()));
    }
    if snapshot.expired {
        return Err(ChannelError::Adapter("poll has expired".to_string()));
    }
    if !snapshot.multi_select && selected_answers.len() > 1 {
        return Err(ChannelError::Adapter(
            "poll allows only one answer".to_string(),
        ));
    }
    if selected_answers
        .iter()
        .any(|index| usize::from(*index) >= snapshot.answer_count)
    {
        return Err(ChannelError::Adapter(
            "selected answer is out of range".to_string(),
        ));
    }
    Ok(())
}

/// `poll_id`'s state, aggregated from `channel_id`'s outbox entries.
async fn load_poll_snapshot<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    poll_id: [u8; 16],
) -> Result<PollSnapshot, ChannelError> {
    let entries = deps.read_outbox_entries(community_id, channel_id).await;
    let poll = crate::history::poll_tallies(&entries)
        .remove(&poll_id)
        .ok_or_else(|| ChannelError::Adapter("poll not found".to_string()))?;
    Ok(PollSnapshot {
        answer_count: poll.answers.len(),
        multi_select: poll.multi_select,
        expired: poll.expires_at.is_some_and(|ts| ts <= deps.now_secs()),
        closed: poll.closed_at.is_some(),
        latest_votes: poll.votes.into_iter().collect(),
        counted: poll.vote_seqs.into_iter().collect(),
        author: poll.author,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_poll_create_rejects_empty_question() {
        let err = validate_poll_create("   ", &["a".into(), "b".into()]).unwrap_err();
        assert!(matches!(err, ChannelError::Adapter(msg) if msg.contains("empty")));
    }

    #[test]
    fn validate_poll_create_rejects_too_few_answers() {
        let err = validate_poll_create("q", &["only".into()]).unwrap_err();
        assert!(matches!(err, ChannelError::Adapter(msg) if msg.contains("2 and 10")));
    }

    #[test]
    fn validate_poll_create_rejects_too_many_answers() {
        let answers: Vec<String> = (0..11).map(|i| format!("a{i}")).collect();
        let err = validate_poll_create("q", &answers).unwrap_err();
        assert!(matches!(err, ChannelError::Adapter(msg) if msg.contains("2 and 10")));
    }

    #[test]
    fn validate_poll_create_rejects_empty_answer() {
        let err = validate_poll_create("q", &["a".into(), "  ".into()]).unwrap_err();
        assert!(matches!(err, ChannelError::Adapter(msg) if msg.contains("empty")));
    }

    #[test]
    fn validate_poll_create_accepts_valid_input() {
        let result = validate_poll_create("Best color?", &["red".into(), "blue".into()]);
        assert!(result.is_ok());
    }

    #[test]
    fn parse_poll_id_rejects_short_hex() {
        let err = parse_poll_id("abcd").unwrap_err();
        assert!(matches!(err, ChannelError::Adapter(_)));
    }

    #[test]
    fn parse_poll_id_accepts_32_hex_chars() {
        let id = parse_poll_id("00112233445566778899aabbccddeeff").unwrap();
        assert_eq!(
            id,
            [
                0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
                0xee, 0xff
            ]
        );
    }

    #[test]
    fn dedupe_selected_answers_sorts_and_dedups() {
        assert_eq!(dedupe_selected_answers(vec![2, 0, 1, 0, 2]), vec![0, 1, 2]);
    }

    #[test]
    fn a_poll_id_is_bound_to_its_creator() {
        let nonce = [9u8; 16];
        let ann = poll_id_of(&PseudonymKey([1; 32]), &nonce);
        assert_eq!(ann, poll_id_of(&PseudonymKey([1; 32]), &nonce));
        assert_ne!(ann, poll_id_of(&PseudonymKey([2; 32]), &nonce));
    }

    /// A close over 1,000 voters (each at the largest seq) is parts that
    /// each fit one entry, and a reader that has read every part counts
    /// every voter; one part short, the poll is still open.
    #[test]
    fn a_close_past_one_page_is_parts_that_each_fit() {
        use crate::deps::OutboxItem;
        use crate::outbox_chain::{seal_batch, PendingEntry};
        use rekindle_codec::community::channel_record::{
            ChannelPollClose, ChannelPollCreate, ChannelPollVote, ChannelRecordEntry,
        };
        let creator = PseudonymKey([1; 32]);
        let nonce = [5u8; 16];
        let poll_id = poll_id_of(&creator, &nonce);
        let voter = |i: u16| {
            let mut key = [0u8; 32];
            key[..2].copy_from_slice(&i.to_be_bytes());
            key[31] = 0xee;
            PseudonymKey(key)
        };
        let counted: Vec<(String, u64)> = (0..1000)
            .map(|i| (hex::encode(voter(i).0), u64::MAX))
            .collect();
        let parts = poll_close_parts(poll_id, &counted).unwrap();
        assert!(parts.len() > 1);
        let author = rekindle_secrets::ed25519_dalek::SigningKey::from_bytes(&[3; 32]);
        let mut entries = vec![OutboxItem {
            author: creator.clone(),
            stream_seq: 1,
            committed_at: 1,
            entry: ChannelRecordEntry::PollCreate(ChannelPollCreate {
                nonce,
                message_id: "m".into(),
                question: "q".into(),
                answers: vec!["a".into(), "b".into()],
                multi_select: false,
                expires_at: None,
                lamport: 1,
            }),
        }];
        entries.extend((0..1000).map(|i| OutboxItem {
            author: voter(i),
            stream_seq: u64::MAX,
            committed_at: 2,
            entry: ChannelRecordEntry::PollVote(ChannelPollVote {
                poll_id,
                selected_answers: vec![0],
                lamport: 1,
            }),
        }));
        for (seq, part) in (2u64..).zip(parts) {
            let PostEntry::PollClose {
                close_id,
                part,
                parts,
                counted,
                ..
            } = part
            else {
                panic!("a close part");
            };
            let entry = ChannelRecordEntry::PollClose(ChannelPollClose {
                poll_id,
                lamport: seq,
                close_id,
                part,
                parts,
                counted: counted_entries(&counted),
            });
            let pending = PendingEntry {
                channel_tag: [7; 16],
                stream_seq: u64::MAX,
                generation: u64::MAX,
                entry: entry.clone(),
            };
            seal_batch(None, &"a".repeat(64), &author, u64::MAX, &[pending])
                .expect("each part fits one entry");
            entries.push(OutboxItem {
                author: creator.clone(),
                stream_seq: seq,
                committed_at: 3,
                entry,
            });
        }
        let short = &entries[..entries.len() - 1];
        assert!(crate::history::poll_tallies(short)[&poll_id]
            .closed_at
            .is_none());
        let poll = &crate::history::poll_tallies(&entries)[&poll_id];
        assert!(poll.closed_at.is_some());
        assert_eq!(poll.votes.len(), 1000);
    }
}
