//! Duplication gates for the code the Rust gates cannot see.
//!
//! `check_duplicate_bodies` and `check_duplicate_constants` in `main.rs`
//! cover Rust function bodies and `SCREAMING_CASE` constants. Three
//! classes fell outside them, and two of the three produced real
//! defects during the v2.0 migration:
//!
//! * **Duplicate type names across crates.** Two `MemberPresence`
//!   structs (one carrying `is_coordinator`, the concept v2.0 deleted)
//!   and two `PeerInfo` structs lived in the workspace at once. The body
//!   gate compares function bodies and these share none; the constant
//!   gate is keyed on constant names. Both were found by hand.
//! * **Frontend duplication.** `src/` had no duplication gate at all —
//!   dependency-cruiser checks *dependencies*, not repetition. 8,000
//!   lines of CSS and 30,000 of TypeScript were unguarded.
//! * **CSS declaration blocks.** The 600-line ceiling made
//!   `xfire-theme.css` split into 20 files; nothing then stopped the
//!   same three-property rule being written into four of them.
//!
//! These checks are deliberately conservative — they compare exact
//! normalised text, so a false positive means two things really are
//! byte-identical and someone should say why in an exception entry.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};

use crate::{crate_of, normalise_body, walk_source_files};

/// One recorded duplicate: a stable key plus the report lines.
pub struct Offender {
    pub key: String,
    pub lines: Vec<String>,
}

/// Accepted duplications, one key per line.
///
/// Same contract as `.dependency-cruiser-known-violations.json`: this is
/// a **burn-down, not an excuse**. No gate's severity was lowered to
/// create it, every entry is a real duplicate, and a duplicate that is
/// not in here fails CI. Shrink it by fixing duplicates and running
/// `cargo xtask baseline-duplication`.
const BASELINE_FILE: &str = "xtask/known-duplication.txt";

fn baseline_path(root: &Path) -> PathBuf {
    root.join(BASELINE_FILE)
}

pub fn load_baseline(root: &Path) -> BTreeSet<String> {
    std::fs::read_to_string(baseline_path(root))
        .map(|t| {
            t.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Filter collected offenders against the baseline, print what is left,
/// and fail if anything survives.
pub fn report(label: &str, offenders: Vec<Offender>, root: &Path, hint: &str) -> Result<()> {
    let baseline = load_baseline(root);
    let fresh: Vec<Offender> = offenders
        .into_iter()
        .filter(|o| !baseline.contains(&o.key))
        .collect();
    if fresh.is_empty() {
        return Ok(());
    }
    for o in &fresh {
        for l in &o.lines {
            println!("{l}");
        }
    }
    Err(anyhow!(
        "{} new {label} duplication(s).\n{hint}\n\
         If the duplication is deliberate, record it with \
         `cargo xtask baseline-duplication` and say why in the commit.",
        fresh.len()
    ))
}

/// Rewrite the baseline from the current tree.
pub fn write_baseline(root: &Path) -> Result<()> {
    let mut keys: BTreeSet<String> = BTreeSet::new();
    for o in crate::collect_duplicate_bodies(root)? {
        keys.insert(o.key);
    }
    for o in collect_duplicate_types(root)? {
        keys.insert(o.key);
    }
    for o in collect_frontend_duplication(root)? {
        keys.insert(o.key);
    }
    let mut out = String::from(
        "# Accepted duplications — a burn-down, not an excuse.\n\
         #\n\
         # Every line is a real duplicate that CI would otherwise fail on.\n\
         # No gate's severity was lowered to produce this file. A duplicate\n\
         # NOT listed here fails the build. Shrink the list by fixing\n\
         # duplicates, then run `cargo xtask baseline-duplication`.\n\
         #\n\
         # Prefer a written exception in xtask/src/duplication.rs (or\n\
         # DUPLICATE_BODY_EXCEPTIONS in main.rs) when the duplication is\n\
         # deliberate and permanent — those carry a reason; this file does not.\n",
    );
    for k in &keys {
        out.push_str(k);
        out.push('\n');
    }
    std::fs::write(baseline_path(root), out).context("writing duplication baseline")?;
    println!("  wrote {} entries to {BASELINE_FILE}", keys.len());
    Ok(())
}

/// A TS body must be at least this long to count. Short arrow functions
/// (`() => setOpen(false)`) collide constantly and mean nothing.
const MIN_TS_BODY_CHARS: usize = 120;

/// A CSS rule needs at least this many declarations before a repeat is
/// worth reporting. One shared `background:` across ten selectors is
/// theming; three-plus identical declarations is a missing utility.
const MIN_CSS_DECLS: usize = 3;

/// Type names that may legitimately exist in more than one crate.
///
/// Keyed on the exact `crate::Type` site list, like the body gate: a
/// bare name would exempt every future collision under that name too.
const DUPLICATE_TYPE_EXCEPTIONS: &[(&[&str], &str)] = &[
    // ── Per-crate error types ────────────────────────────────────
    // Rust API Guidelines, "Error types are meaningful and
    // well-behaved": *"Define a meaningful error type specific to your
    // crate or to the individual function."* Same-name error enums in
    // different crates are the prescribed practice, not a collision —
    // each crate owns its namespace.
    (
        &[
            "rekindle-crypto::CryptoError",
            "rekindle-types::CryptoError",
        ],
        "Tier 1 owns the seven shared conditions; rekindle-crypto WRAPS \
         them as `Core(#[from] CoreCryptoError)` and adds four \
         Signal-specific ones. The wrap is the point — a caller can \
         handle either level.",
    ),
    (
        &[
            "rekindle-gossip::GossipError",
            "rekindle-types::GossipError",
        ],
        "Same shape as CryptoError: Tier 1 names the shared conditions \
         (broadcast failure, rate limiting, envelope validity) and \
         rekindle-gossip wraps them as `Core(#[from] …)` alongside its \
         own three. Before this branch both were in scope in that crate \
         at once, returned by different modules, with only one exported.",
    ),
    (
        &["rekindle-calls::CallError", "rekindle-transport::CallError"],
        "rekindle-calls owns call-domain errors (key derivation, state \
         machine); transport's covers its own call *operations* (queue, \
         store, serialize). Neither is a superset of the other and \
         neither crosses the other's boundary.",
    ),
    // ── Same name, genuinely different concepts ──────────────────
    (
        &["rekindle-audit::AuditEntry", "rekindle-node::AuditEntry"],
        "rekindle-audit's is a hash-chain link — `cursor`, `prev_mac`, \
         `mac`, `record`. rekindle-node's is the IPC audit record the \
         chain carries — sequence, timestamps, sender name, security \
         level, event type. One is the envelope, one is the contents.",
    ),
    (
        &[
            "rekindle-channel::AutoModAction",
            "rekindle-protocol::AutoModAction",
        ],
        "protocol's is the configured *action* a rule takes \
         (BlockMessage, AlertModerators{channel_id}, \
         TimeoutMember{duration}, LogOnly). channel's is the local \
         *outcome* of evaluating a message (Allow, BlockLocally, \
         BlurContent, AlertModerators). Rule config versus evaluation \
         result — merging them would make an unrepresentable state \
         representable.",
    ),
    (
        &["rekindle-cli::ChannelEntry", "rekindle-types::ChannelEntry"],
        "Tier 1's is the channel *log* entry — the enum of things \
         written to a channel record (Message with MEK ciphertext, \
         reactions, edits). The CLI's is a TUI row: id, name, kind, \
         category, unread count, sort order. Unrelated beyond the word \
         'channel'; the CLI's should probably be `ChannelTreeRow`.",
    ),
    (
        &["rekindle (src-tauri)::Message", "rekindle-node::Message"],
        "rekindle-node's is the generic IPC envelope `Message<T>` — \
         wire version, UUIDv7 id, correlation id, dual clock, \
         classification. src-tauri's is a chat message DTO for the \
         frontend. Same word, different layers of the stack.",
    ),
    (
        &[
            "rekindle (src-tauri)::SharedState",
            "rekindle-transport::SharedState",
        ],
        "transport's is a struct of atomics tracking Veilid attachment, \
         peer counts and latency. src-tauri's is \
         `type SharedState = Arc<AppState>`. Not comparable.",
    ),
    (
        &[
            "rekindle (src-tauri)::PendingFriendRequest",
            "rekindle-transport::PendingFriendRequest",
        ],
        "Two stages of one flow. transport's is what arrives over the \
         wire — profile DHT key and route blob, needed to answer. \
         src-tauri's is what the UI lists — public key, display name, \
         message, `received_at`. Merging would put routing data in a \
         view model.",
    ),
    // ── Codec boundaries: domain type vs its wire DTO ─────────────
    (
        &[
            "rekindle-crypto::PreKeyBundle",
            "rekindle-protocol::PreKeyBundle",
        ],
        "crypto's is the domain type; protocol's is its Cap'n Proto DTO. \
         NOTE, from the X3DH specification: the bundle omits the \
         one-time prekey when the server has none left, and the DH count \
         differs (four operations with it, three without). crypto's \
         models that with `Option`; the capnp DTO collapses the ids to \
         bare `u32`, so `None` and a real id 0 are indistinguishable. \
         Not live — that DTO is written into conversation headers and \
         never read back for key agreement — but it is a landmine in a \
         codec someone would reach for.",
    ),
    (
        &[
            "rekindle-protocol::MekTransferPayload",
            "rekindle-transport::MekTransferPayload",
        ],
        "Two wire forms. protocol's is the Cap'n Proto envelope payload \
         (`community_id`, `channel_id: Option`, `sender_pseudonym`); \
         transport's is the local RPC form (`channel_id: String`, \
         `rotator_pseudonym_hex`). `payload/rpc.rs` already documents \
         which is on which wire, and the bare-envelope path deliberately \
         carries protocol's.",
    ),
    (
        &[
            "rekindle (src-tauri)::RoleDto",
            "rekindle-protocol::RoleDto",
        ],
        "src-tauri's serialises `permissions` as a string \
         (`serialize_u64_as_string`): a u64 above 2^53-1 loses low bits \
         through JavaScript's Number, which silently strips \
         ADMINISTRATOR (bit 3) from the Owner role. protocol's is the \
         Rust-to-Rust form with no such constraint.",
    ),
    (
        &[
            "rekindle-crypto::SignalSessionManager",
            "rekindle-transport::SignalSessionManager",
        ],
        "Two shells over the *same* primitives — both use \
         `rekindle_crypto::signal::{pqxdh, ratchet}`, so cross-track \
         compatibility holds by construction. They differ in store \
         backing (Stronghold/vault versus the daemon's own \
         `signal_store`) and therefore in method shape: crypto's is \
         async with a `SessionCache` of per-peer `tokio::sync::Mutex`es, \
         transport's synchronous with a `parking_lot` map. Both now \
         serialise the ratchet per peer, which the Double Ratchet spec \
         requires — see the comment on `peer_locks`.",
    ),
    // ── One finding, three names: the two frontends have parallel
    //    view and event vocabularies. Recorded as exceptions because
    //    converging them is an architecture change, not a rename.
    (
        &[
            "rekindle (src-tauri)::CommunityDetail",
            "rekindle-types::CommunityDetail",
        ],
        "Tier 1's is what the CLI renders; src-tauri's is what the Tauri \
         frontend renders, and carries icon/banner hashes the CLI has no \
         use for. See the PresenceEvent entry — same finding.",
    ),
    (
        &[
            "rekindle (src-tauri)::PresenceEvent",
            "rekindle-types::PresenceEvent",
        ],
        "THE FINDING, recorded once here. src-tauri never references \
         `SubscriptionEvent` at all: the Tauri frontend gets \
         `channels::presence_channel::PresenceEvent` \
         (FriendOnline/FriendOffline/StatusChanged/GameChanged, \
         Serialize-only) while the CLI gets \
         `rekindle_types::subscription_events::PresenceEvent` \
         (CommunityMemberChanged/FriendChanged, round-trippable). Two \
         event vocabularies, one per frontend — which contradicts \
         'the daemon is the substrate, frontends are interchangeable'. \
         Converging them means routing src-tauri's ~150 emit sites \
         through the subscription stream; that is an architecture \
         change with its own plan, not a duplicate to collapse here.",
    ),
    (
        &[
            "rekindle (src-tauri)::VoiceEvent",
            "rekindle-types::VoiceEvent",
        ],
        "Same finding as PresenceEvent — see that entry.",
    ),
    (
        &["rekindle-protocol::GameInfo", "rekindle-types::GameInfo"],
        "Two different wire forms, not one type in two places. \
         protocol's is the 1:1 rich-presence payload the Cap'n Proto \
         codec encodes — `game_id: u32`, `elapsed_seconds: u32`, a \
         `server_info` field, snake_case on the wire. Tier 1's is the \
         community presence form — `game_id: Option<String>`, \
         `elapsed_seconds: Option<u64>`, no `server_info`, camelCase. \
         Converging them is a Cap'n Proto presence schema change, not a \
         rename; recorded here so the next reader sees the difference \
         is deliberate rather than assuming they are interchangeable.",
    ),
    (
        &["rekindle-files::MockCalls", "rekindle-video::MockCalls"],
        "Per-crate test doubles: same idiom, different domain fields. \
         The migration plan examined these and declined them by name.",
    ),
    (
        &["rekindle-files::MockDeps", "rekindle-video::MockDeps"],
        "Same as MockCalls — a shared mock would couple two unrelated \
         Deps traits so one crate's test could not move without the \
         other's.",
    ),
    (
        &[
            "rekindle-governance-runtime::CommunityMembership",
            "rekindle-transport::CommunityMembership",
        ],
        "Deliberate, and documented in the migration plan (2.3): \
         transport's is the daemon's persisted session record; the \
         runtime crate's is the all-Option snapshot that crosses the \
         Deps boundary. Converging them would drag host-shaped state \
         across the adapter horizon, which is the opposite of the \
         services-pattern design.",
    ),
];

/// Frontend duplicates that are accepted rather than fixed.
///
/// Keyed on the sorted `file:name` sites.
const DUPLICATE_FRONTEND_EXCEPTIONS: &[(&[&str], &str)] = &[];

// ────────────────────────────────────────────────────────────────
// check-duplicate-types
// ────────────────────────────────────────────────────────────────

/// Extract `(name, line)` for every `struct` / `enum` / `type` alias
/// declared in a Rust file, skipping anything after `#[cfg(test)]`.
fn types_in(src: &str) -> Vec<(String, usize)> {
    let cut = src.find("#[cfg(test)]").unwrap_or(src.len());
    let src = &src[..cut];
    let mut out = Vec::new();

    // Only module-level declarations count. `type Err = …` inside a
    // `impl FromStr` is an associated type, not a type this crate owns,
    // and every crate with a `FromStr` impl would otherwise collide with
    // every other on the name `Err`.
    let mut depth = 0i32;

    for (idx, line) in src.lines().enumerate() {
        let t = line.trim_start();
        let at_module_level = depth == 0;
        depth += i32::try_from(line.matches('{').count()).unwrap_or(0)
            - i32::try_from(line.matches('}').count()).unwrap_or(0);
        if !at_module_level {
            continue;
        }
        // Only *declarations*: `pub struct Foo`, `enum Bar`, `type Baz =`.
        // A `pub use` re-export names the same type and must not count as
        // a second declaration — that is how a facade would be flagged.
        // `pub` only, including `pub(crate)`/`pub(super)`. A private type
        // is invisible outside its crate, so it cannot be confused with
        // another crate's — `struct CachedRoute` in transport's peer
        // registry and `struct InviteContext` in the join flow are
        // local details, not competing definitions.
        let vis = t
            .strip_prefix("pub(crate) ")
            .or_else(|| t.strip_prefix("pub(super) "))
            .or_else(|| t.strip_prefix("pub "));
        let Some(vis) = vis else { continue };
        let rest = ["struct ", "enum ", "type "]
            .iter()
            .find_map(|kw| vis.strip_prefix(kw));
        let Some(rest) = rest else { continue };

        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        // Skip single-letter generics-ish names and empty matches.
        if name.len() < 3 || !name.starts_with(char::is_uppercase) {
            continue;
        }
        // `pub type Result<T> = std::result::Result<T, ThisCrateError>`
        // is the idiomatic per-crate alias. Every crate is *supposed* to
        // have one; flagging them would train people to ignore the gate.
        if name == "Result" {
            continue;
        }
        out.push((name, idx + 1));
    }
    out
}

/// Fail when one type name is *declared* in two or more crates.
///
/// This is the gate that would have caught `MemberPresence` and
/// `PeerInfo`. Re-exports (`pub use other::Thing`) are not declarations,
/// so a façade — the correct way to share a type — stays silent.
pub fn check_duplicate_types(root: &Path) -> Result<()> {
    report(
        "type-name",
        collect_duplicate_types(root)?,
        root,
        "Two structs sharing a name and a role will drift, and no body- or \
         constant-based gate can see it: that is how a `MemberPresence` \
         carrying `is_coordinator` survived the coordinator's removal.\n\
         Move the type to a crate both can reach and re-export it — a \
         `pub use` facade is not a declaration, so it stays silent here.",
    )
}

pub fn collect_duplicate_types(root: &Path) -> Result<Vec<Offender>> {
    let mut by_name: BTreeMap<String, Vec<(String, String, usize)>> = BTreeMap::new();

    for subdir in ["crates", "src-tauri/src"] {
        for path in walk_source_files(&root.join(subdir), &["rs"])? {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .display()
                .to_string();
            if rel.contains("/tests/") || rel.contains("/benches/") {
                continue;
            }
            let Ok(src) = std::fs::read_to_string(&path) else {
                continue;
            };
            for (name, line) in types_in(&src) {
                by_name
                    .entry(name)
                    .or_default()
                    .push((crate_of(&rel), rel.clone(), line));
            }
        }
    }

    let mut offenders = Vec::new();
    for (name, sites) in &by_name {
        let crates: std::collections::BTreeSet<&str> =
            sites.iter().map(|(c, _, _)| c.as_str()).collect();
        if crates.len() < 2 {
            continue;
        }
        let key: Vec<String> = crates.iter().map(|c| format!("{c}::{name}")).collect();
        if DUPLICATE_TYPE_EXCEPTIONS.iter().any(|(allowed, _)| {
            allowed.len() == key.len() && allowed.iter().zip(key.iter()).all(|(a, k)| *a == k)
        }) {
            continue;
        }
        let mut lines = vec![format!("  ✗ `{name}` declared in {} crates", crates.len())];
        for (crate_name, rel, line) in sites {
            lines.push(format!("      {crate_name:28} {rel}:{line}"));
        }
        offenders.push(Offender {
            key: format!("type {}", key.join(" ")),
            lines,
        });
    }

    Ok(offenders)
}

// ────────────────────────────────────────────────────────────────
// check-frontend-duplication
// ────────────────────────────────────────────────────────────────

/// Extract `(name, normalised_body, line)` for TS/TSX functions —
/// `function foo(…) {…}`, `const foo = (…) => {…}`, and object methods.
fn functions_in_ts(src: &str) -> Vec<(String, String, usize)> {
    let mut out = Vec::new();

    for (idx, line) in src.lines().enumerate() {
        let t = line.trim_start();
        let name = parse_ts_fn_name(t);
        let Some(name) = name else { continue };

        // Take the brace-balanced body starting at this line.
        let start = src.lines().take(idx).map(|l| l.len() + 1).sum::<usize>();
        let Some(open_rel) = src[start..].find('{') else {
            continue;
        };
        let open = start + open_rel;
        let mut depth = 0usize;
        let mut close = open;
        for (i, c) in src[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        close = open + i;
                        break;
                    }
                }
                _ => {}
            }
        }
        if close <= open {
            continue;
        }
        let body = normalise_body(&src[open + 1..close]);
        if body.len() >= MIN_TS_BODY_CHARS {
            out.push((name, body, idx + 1));
        }
    }
    out
}

/// `function foo(` / `const foo = (` / `export async function foo(`.
fn parse_ts_fn_name(t: &str) -> Option<String> {
    let after_fn = t
        .strip_prefix("export async function ")
        .or_else(|| t.strip_prefix("export function "))
        .or_else(|| t.strip_prefix("async function "))
        .or_else(|| t.strip_prefix("function "));
    if let Some(rest) = after_fn {
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        return (!name.is_empty()).then_some(name);
    }
    // `const foo = (…) => {` / `const foo = async (…) => {`
    let rest = t
        .strip_prefix("export const ")
        .or_else(|| t.strip_prefix("const "))?;
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() {
        return None;
    }
    let after = rest.get(name.len()..)?;
    (after.contains("=>") || after.contains("= function")).then_some(name)
}

/// Extract `(normalised_declarations, line)` for every innermost CSS
/// rule — a `{…}` containing no nested braces, so `@media` wrappers
/// contribute their inner rules rather than one giant blob.
fn css_blocks_in(src: &str) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let bytes = src.as_bytes();
    let mut i = 0usize;

    while i < bytes.len() {
        if bytes[i] != b'{' {
            i += 1;
            continue;
        }
        let open = i;
        let mut j = open + 1;
        let mut nested = false;
        while j < bytes.len() && bytes[j] != b'}' {
            if bytes[j] == b'{' {
                nested = true;
                break;
            }
            j += 1;
        }
        if nested || j >= bytes.len() {
            i = open + 1;
            continue;
        }
        let inner = &src[open + 1..j];
        let mut decls: Vec<String> = inner
            .split(';')
            .map(|d| d.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|d| !d.is_empty() && d.contains(':') && !d.starts_with("/*"))
            .collect();
        if decls.len() >= MIN_CSS_DECLS {
            decls.sort();
            let line = src[..open].matches('\n').count() + 1;
            out.push((decls.join(" ; "), line));
        }
        i = j + 1;
    }
    out
}

/// Fail on duplicated TS function bodies and duplicated CSS rules.
///
/// The frontend had no duplication gate of any kind; `src/` is 31k lines
/// of TypeScript and 8k of CSS, all of it previously unguarded.
pub fn check_frontend_duplication(root: &Path) -> Result<()> {
    report(
        "frontend",
        collect_frontend_duplication(root)?,
        root,
        "TypeScript: hoist the shared body to src/utils/ (a leaf every tier \
         may import) and call it from both sites.\n\
         CSS: give the rule one home — group the selectors into a single \
         rule, or promote it to a utility class. Repeating three \
         declarations in four files is how a hover state ends up fixed in \
         three of them.",
    )
}

pub fn collect_frontend_duplication(root: &Path) -> Result<Vec<Offender>> {
    let src_dir = root.join("src");
    let mut offenders: Vec<Offender> = Vec::new();

    // ── TypeScript ────────────────────────────────────────────────
    let mut ts_by_body: BTreeMap<String, Vec<(String, String, usize)>> = BTreeMap::new();
    for path in walk_source_files(&src_dir, &["ts", "tsx"])? {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (name, body, line) in functions_in_ts(&text) {
            ts_by_body
                .entry(body)
                .or_default()
                .push((rel.clone(), name, line));
        }
    }
    for sites in ts_by_body.values() {
        let files: std::collections::BTreeSet<&str> =
            sites.iter().map(|(f, _, _)| f.as_str()).collect();
        if files.len() < 2 {
            continue;
        }
        let mut key: Vec<String> = sites.iter().map(|(f, n, _)| format!("{f}:{n}")).collect();
        key.sort();
        key.dedup();
        if DUPLICATE_FRONTEND_EXCEPTIONS.iter().any(|(allowed, _)| {
            allowed.len() == key.len() && allowed.iter().zip(key.iter()).all(|(a, k)| *a == k)
        }) {
            continue;
        }
        let mut lines = vec![format!("  ✗ identical TS body in {} files", files.len())];
        for (rel, name, line) in sites {
            lines.push(format!("      {name:32} {rel}:{line}"));
        }
        offenders.push(Offender {
            key: format!("ts {}", key.join(" ")),
            lines,
        });
    }

    // ── CSS ───────────────────────────────────────────────────────
    let mut css_by_block: BTreeMap<String, Vec<(String, usize)>> = BTreeMap::new();
    for path in walk_source_files(&src_dir, &["css"])? {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (block, line) in css_blocks_in(&text) {
            css_by_block
                .entry(block)
                .or_default()
                .push((rel.clone(), line));
        }
    }
    for (block, sites) in &css_by_block {
        if sites.len() < 2 {
            continue;
        }
        let mut lines = vec![
            format!("  ✗ identical CSS rule at {} sites", sites.len()),
            format!("      {{ {block} }}"),
        ];
        for (rel, line) in sites {
            lines.push(format!("      {rel}:{line}"));
        }
        // Keyed on the rule text, not the sites: moving one of four
        // copies into a new file must not silently re-accept it.
        offenders.push(Offender {
            key: format!("css {block}"),
            lines,
        });
    }

    Ok(offenders)
}
