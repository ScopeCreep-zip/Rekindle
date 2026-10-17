//! Rendering and proving the host's intent reviews (plan D3.4, U1.7;
//! ADR 0019). The CLI sends every request as it is; when the host's gate
//! holds one it answers `IntentRequired`, and [`send`] renders the review
//! and commits it with the proof its class asks for. The CLI never decides
//! whether to ask.
//!
//! Proofs: Commit is a y/N prompt (default No) or `--yes`; Acknowledge
//! states the fact, then `yes` typed or `--confirm=<subject>`; TypeSubject
//! is the subject's name typed or `--confirm=<subject>`; Passphrase comes
//! from the passphrase reader (a terminal, `--passphrase-file`, stdin;
//! never a flag value or the environment). `--yes` never satisfies a
//! trust-changing, destructive or protected review, and without a terminal
//! a missing proof is an error naming the flag.

use std::path::PathBuf;

use anyhow::Context;
use rekindle_client::{ClientError, DaemonClient, Ran};
use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::intent::{IntentProof, IntentReview, ProofRequired, Reversibility};
use zeroize::Zeroizing;

/// What the command line offers as proof.
#[derive(Default)]
struct Flags {
    yes: bool,
    confirm: Option<String>,
    passphrase_file: Option<PathBuf>,
    /// A passphrase the command already read for its request (rotate,
    /// export), so the user is not asked twice.
    passphrase: Option<Zeroizing<String>>,
    structured: bool,
}

static FLAGS: parking_lot::Mutex<Option<Flags>> = parking_lot::Mutex::new(None);

fn with_flags<T>(f: impl FnOnce(&mut Flags) -> T) -> T {
    let mut guard = FLAGS.lock();
    f(guard.get_or_insert_with(Flags::default))
}

/// The global proof flags (`--yes`, `--confirm`) and the output mode.
pub fn set_proof_flags(yes: bool, confirm: Option<String>, structured: bool) {
    with_flags(|flags| {
        flags.yes = yes;
        flags.confirm = confirm;
        flags.structured = structured;
    });
}

/// Where a passphrase proof is read from, for the command that takes one.
pub fn set_passphrase_file(path: Option<PathBuf>) {
    with_flags(|flags| flags.passphrase_file = path);
}

/// A passphrase the command already read, offered as the proof.
pub fn offer_passphrase(passphrase: Zeroizing<String>) {
    with_flags(|flags| flags.passphrase = Some(passphrase));
}

/// Send `request`; if the host holds it for review, render the review,
/// prove it and commit, as often as the host asks again (a fact changed).
///
/// # Errors
/// The request failed or was refused, or the proof could not be given.
pub async fn send(client: &DaemonClient, request: IpcRequest) -> anyhow::Result<serde_json::Value> {
    let mut answer = client.request_outcome(request).await;
    let mut reviewed = false;
    loop {
        match answer {
            Ok(ran) => {
                render_reversibility(&ran, reviewed);
                return Ok(ran.data);
            }
            Err(ClientError::IntentRequired(review)) => {
                reviewed = true;
                let proof = prove(&review)?;
                answer = client.commit_intent(review.intent_id, proof).await;
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// Render `review` and obtain the proof it asks for.
fn prove(review: &IntentReview) -> anyhow::Result<IntentProof> {
    render(review)?;
    let subject = review.subject.typed_name().to_owned();
    let (yes, confirm, passphrase_file, offered) = with_flags(|flags| {
        (
            flags.yes,
            flags.confirm.clone(),
            flags.passphrase_file.clone(),
            flags.passphrase.clone(),
        )
    });
    match review.proof {
        ProofRequired::None => Ok(IntentProof::Commit),
        ProofRequired::Commit => {
            if yes || super::ask_yes_no(&format!("{}?", verb(review)), "--yes")? {
                Ok(IntentProof::Commit)
            } else {
                anyhow::bail!("cancelled")
            }
        }
        ProofRequired::Acknowledge => {
            if confirm.as_deref() == Some(subject.as_str()) {
                return Ok(IntentProof::Acknowledged);
            }
            let typed = super::ask_text(
                "Type \"yes\" to acknowledge",
                &format!("--confirm={subject}"),
            )?;
            if typed.trim() == "yes" {
                Ok(IntentProof::Acknowledged)
            } else {
                anyhow::bail!("cancelled")
            }
        }
        ProofRequired::TypeSubject => {
            if let Some(given) = confirm {
                return Ok(IntentProof::Typed(given));
            }
            let typed = super::ask_text(
                &format!("Type \"{subject}\" to confirm"),
                &format!("--confirm={subject}"),
            )?;
            Ok(IntentProof::Typed(typed.trim().to_owned()))
        }
        ProofRequired::Passphrase => match offered {
            Some(passphrase) => Ok(IntentProof::Passphrase(passphrase)),
            None => Ok(IntentProof::Passphrase(super::read_passphrase(
                "Passphrase to confirm",
                passphrase_file.as_deref(),
            )?)),
        },
    }
}

fn verb(review: &IntentReview) -> String {
    format!("{:?} {}", review.kind, review.subject.typed_name())
}

/// The review on stderr: as text, or as the JSON object with
/// `--output json`, so stdout keeps only the result.
fn render(review: &IntentReview) -> anyhow::Result<()> {
    use std::io::Write as _;
    let structured = with_flags(|flags| flags.structured);
    let mut err = std::io::stderr().lock();
    if structured {
        let json = serde_json::to_string(review).context("review did not serialize")?;
        writeln!(err, "{json}")?;
        return Ok(());
    }
    writeln!(
        err,
        "{} needs confirmation ({:?}):",
        verb(review),
        review.class
    )?;
    for fact in &review.facts {
        writeln!(err, "  - {fact:?}")?;
    }
    for escalation in &review.escalations {
        writeln!(err, "  ! {escalation:?}")?;
    }
    writeln!(
        err,
        "  reversibility: {:?}; notifies: {:?}",
        review.reversibility, review.notifies
    )?;
    Ok(())
}

/// How a run can be taken back: after a review, and for a Routine run with
/// an undo.
fn render_reversibility(ran: &Ran, reviewed: bool) {
    use std::io::Write as _;
    let Some(reversibility) = ran.reversibility else {
        return;
    };
    let worth = reviewed
        || matches!(
            reversibility,
            Reversibility::Undoable | Reversibility::ReversibleByInverse(_)
        );
    if worth && !with_flags(|flags| flags.structured) {
        let _ = writeln!(std::io::stderr(), "reversibility: {reversibility:?}");
    }
}
