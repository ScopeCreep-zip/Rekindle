//! Golden JSON shape of every `SubscriptionEvent` the webview receives.
//!
//! The desktop delivers each event as `serde_json::to_value(&SubscriptionEvent)`
//! (externally tagged), and the TypeScript side parses those keys by
//! hand. This test pins three things (plan C7.26 item 2):
//!
//! 1. **Coverage.** Every family has an exhaustive `variant_name` match
//!    with no wildcard arm, so a new variant does not compile until it is
//!    named, and the coverage tests fail until it has a sample. Nested
//!    enums carried in fields are covered the same way.
//! 2. **Shape.** All samples serialize to exactly the JSON in
//!    `tests/golden/subscription_events.json`. Run with `UPDATE_GOLDEN=1`
//!    to rewrite the file after a deliberate wire change.
//! 3. **Casing.** Every object key in every sample is camelCase. No event
//!    payload carries a free-form map (`HashMap`/`BTreeMap<String, _>`),
//!    so every key is schema and none is exempt.

mod call;
mod channel;
mod crypto;
mod friend;
mod governance;
mod intent;
mod membership;
mod network;
mod notification;
mod presence;
mod social;
mod system;
mod typing;
mod voice;

use std::collections::BTreeSet;
use std::path::PathBuf;

use rekindle_types::subscription_events::{EventCategory, SubscriptionEvent, UnreadContext};
use serde_json::{Map, Value};

// ── Deterministic sample values ─────────────────────────────────────────

/// A 64-hex public key made of one repeated hex digit.
pub fn pk(digit: char) -> String {
    digit.to_string().repeat(64)
}

/// A Veilid record key (`VLD0:` + 43 base64url chars).
pub fn rk(c: char) -> String {
    format!("VLD0:{}", c.to_string().repeat(43))
}

/// A 32-hex id (16-byte UUID) with a prefix, e.g. `msg_` + 32 hex.
pub fn id(prefix: &str, digit: char) -> String {
    format!("{prefix}{}", digit.to_string().repeat(32))
}

/// The community governance key used by every community-scoped sample.
pub fn community() -> String {
    rk('g')
}

/// The channel id used by every channel-scoped sample.
pub fn channel() -> String {
    id("ch_", 'c')
}

pub fn text(s: &str) -> String {
    s.to_owned()
}

/// A sample id paired with its event.
pub type Sample = (String, SubscriptionEvent);

// ── Top-level coverage ──────────────────────────────────────────────────

/// One family: its serde tag, its variant names, and the nested-enum
/// variant names its fields can carry.
struct Family {
    tag: &'static str,
    names: &'static [&'static str],
    nested: &'static [&'static str],
}

const UNREAD_NAMES: &[&str] = &["unreadChanged"];
const UNREAD_NESTED: &[&str] = &[
    "UnreadContext.channel",
    "UnreadContext.dm",
    "UnreadContext.friendRequests",
];

const FAMILIES: &[Family] = &[
    Family {
        tag: "channelMessage",
        names: channel::NAMES,
        nested: &[],
    },
    Family {
        tag: "typing",
        names: typing::NAMES,
        nested: typing::NESTED,
    },
    Family {
        tag: "presence",
        names: presence::NAMES,
        nested: presence::NESTED,
    },
    Family {
        tag: "membership",
        names: membership::NAMES,
        nested: &[],
    },
    Family {
        tag: "friend",
        names: friend::NAMES,
        nested: &[],
    },
    Family {
        tag: "crypto",
        names: crypto::NAMES,
        nested: crypto::NESTED,
    },
    Family {
        tag: "voice",
        names: voice::NAMES,
        nested: voice::NESTED,
    },
    Family {
        tag: "governance",
        names: governance::NAMES,
        nested: &[],
    },
    Family {
        tag: "social",
        names: social::NAMES,
        nested: social::NESTED,
    },
    Family {
        tag: "call",
        names: call::NAMES,
        nested: &[],
    },
    Family {
        tag: "notification",
        names: notification::NAMES,
        nested: &[],
    },
    Family {
        tag: "network",
        names: network::NAMES,
        nested: network::NESTED,
    },
    Family {
        tag: "system",
        names: system::NAMES,
        nested: &[],
    },
    Family {
        tag: "unreadChanged",
        names: UNREAD_NAMES,
        nested: UNREAD_NESTED,
    },
];

/// What one sample exercises: its family tag, its variant name and the
/// nested-enum variants inside it.
struct Classified {
    tag: &'static str,
    variant: &'static str,
    nested: Vec<&'static str>,
}

fn unread_context_name(context: &UnreadContext) -> &'static str {
    match context {
        UnreadContext::Channel { .. } => "UnreadContext.channel",
        UnreadContext::Dm { .. } => "UnreadContext.dm",
        UnreadContext::FriendRequests => "UnreadContext.friendRequests",
    }
}

/// Exhaustive over `SubscriptionEvent`: a new family does not compile
/// until it has an arm here.
fn classify(event: &SubscriptionEvent) -> Classified {
    let (tag, variant, nested) = match event {
        SubscriptionEvent::ChannelMessage(e) => {
            ("channelMessage", channel::variant_name(e), vec![])
        }
        SubscriptionEvent::Typing(e) => {
            ("typing", typing::variant_name(e), typing::nested_names(e))
        }
        SubscriptionEvent::Presence(e) => (
            "presence",
            presence::variant_name(e),
            presence::nested_names(e),
        ),
        SubscriptionEvent::Membership(e) => ("membership", membership::variant_name(e), vec![]),
        SubscriptionEvent::Friend(e) => ("friend", friend::variant_name(e), vec![]),
        SubscriptionEvent::Crypto(e) => {
            ("crypto", crypto::variant_name(e), crypto::nested_names(e))
        }
        SubscriptionEvent::Voice(e) => ("voice", voice::variant_name(e), voice::nested_names(e)),
        SubscriptionEvent::Governance(e) => ("governance", governance::variant_name(e), vec![]),
        SubscriptionEvent::Social(e) => {
            ("social", social::variant_name(e), social::nested_names(e))
        }
        SubscriptionEvent::Call(e) => ("call", call::variant_name(e), vec![]),
        SubscriptionEvent::Notification(e) => {
            ("notification", notification::variant_name(e), vec![])
        }
        SubscriptionEvent::Network(e) => (
            "network",
            network::variant_name(e),
            network::nested_names(e),
        ),
        SubscriptionEvent::System(e) => ("system", system::variant_name(e), vec![]),
        SubscriptionEvent::UnreadChanged { context, .. } => (
            "unreadChanged",
            "unreadChanged",
            vec![unread_context_name(context)],
        ),
    };
    Classified {
        tag,
        variant,
        nested,
    }
}

fn unread_samples() -> Vec<Sample> {
    let ev = |suffix: &str, context: UnreadContext, count: u32| {
        (
            format!("unreadChanged.unreadChanged.{suffix}"),
            SubscriptionEvent::UnreadChanged { context, count },
        )
    };
    vec![
        ev(
            "channel",
            UnreadContext::Channel {
                community: community(),
                channel: channel(),
            },
            3,
        ),
        ev("dm", UnreadContext::Dm { peer_key: pk('b') }, 7),
        ev("friendRequests", UnreadContext::FriendRequests, 1),
    ]
}

fn all_samples() -> Vec<Sample> {
    let mut all = Vec::new();
    all.extend(channel::samples());
    all.extend(typing::samples());
    all.extend(presence::samples());
    all.extend(membership::samples());
    all.extend(friend::samples());
    all.extend(crypto::samples());
    all.extend(voice::samples());
    all.extend(governance::samples());
    all.extend(social::samples());
    all.extend(call::samples());
    all.extend(notification::samples());
    all.extend(network::samples());
    all.extend(system::samples());
    all.extend(unread_samples());
    all
}

fn family(tag: &str) -> &'static Family {
    FAMILIES
        .iter()
        .find(|f| f.tag == tag)
        .unwrap_or_else(|| panic!("no family table row for `{tag}`"))
}

/// The const names are distinct, the samples produce exactly that set,
/// and every nested-enum variant the family can carry appears at least
/// once.
fn check_family(tag: &str) {
    let fam = family(tag);
    let declared: BTreeSet<&str> = fam.names.iter().copied().collect();
    assert_eq!(
        declared.len(),
        fam.names.len(),
        "{tag}: NAMES has duplicates"
    );

    let samples = all_samples();
    let classified: Vec<Classified> = samples
        .iter()
        .map(|(_, e)| classify(e))
        .filter(|c| c.tag == tag)
        .collect();
    let seen: BTreeSet<&str> = classified.iter().map(|c| c.variant).collect();
    assert_eq!(
        seen.len(),
        fam.names.len(),
        "{tag}: distinct sampled variants must equal NAMES.len()"
    );
    let missing: Vec<&&str> = declared.difference(&seen).collect();
    assert!(
        missing.is_empty(),
        "{tag}: variants without a sample: {missing:?}"
    );
    let undeclared: Vec<&&str> = seen.difference(&declared).collect();
    assert!(
        undeclared.is_empty(),
        "{tag}: match arms missing from NAMES: {undeclared:?}"
    );

    let nested_declared: BTreeSet<&str> = fam.nested.iter().copied().collect();
    assert_eq!(
        nested_declared.len(),
        fam.nested.len(),
        "{tag}: NESTED has duplicates"
    );
    let nested_seen: BTreeSet<&str> = classified
        .iter()
        .flat_map(|c| c.nested.iter().copied())
        .collect();
    assert_eq!(
        nested_seen, nested_declared,
        "{tag}: nested enum variants sampled vs declared"
    );
}

// ── JSON helpers ────────────────────────────────────────────────────────

/// Rebuild `value` with every object's keys in sorted order, whatever
/// map type `serde_json` was compiled with.
fn sorted(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            let mut out = Map::new();
            for (k, v) in entries {
                out.insert(k, sorted(v));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sorted).collect()),
        other => other,
    }
}

fn serialized_samples() -> Map<String, Value> {
    let mut entries: Vec<(String, Value)> = all_samples()
        .into_iter()
        .map(|(id, event)| {
            let value = serde_json::to_value(&event).expect("serialize sample");
            (id, sorted(value))
        })
        .chain(
            intent::samples()
                .into_iter()
                .map(|(id, value)| (id, sorted(value))),
        )
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = Map::new();
    for (k, v) in entries {
        out.insert(k, v);
    }
    out
}

fn is_camel_case(key: &str) -> bool {
    let mut chars = key.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase()) && chars.all(|c| c.is_ascii_alphanumeric())
}

fn collect_bad_keys(value: &Value, path: &str, bad: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                let here = format!("{path}.{k}");
                if !is_camel_case(k) {
                    bad.push(here.clone());
                }
                collect_bad_keys(v, &here, bad);
            }
        }
        Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                collect_bad_keys(v, &format!("{path}[{i}]"), bad);
            }
        }
        _ => {}
    }
}

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/subscription_events.json")
}

// ── Tests ───────────────────────────────────────────────────────────────

#[test]
fn every_family_has_samples() {
    let declared: BTreeSet<&str> = FAMILIES.iter().map(|f| f.tag).collect();
    assert_eq!(declared.len(), FAMILIES.len(), "FAMILIES has duplicates");
    assert_eq!(
        FAMILIES.len(),
        EventCategory::ALL.len(),
        "one family per SubscriptionEvent variant"
    );
    let seen: BTreeSet<&str> = all_samples().iter().map(|(_, e)| classify(e).tag).collect();
    assert_eq!(
        seen, declared,
        "SubscriptionEvent variants sampled vs declared"
    );
}

#[test]
fn sample_ids_are_unique_and_name_their_variant() {
    let samples = all_samples();
    let ids: BTreeSet<&str> = samples.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids.len(), samples.len(), "duplicate sample ids");
    for (id, event) in &samples {
        let c = classify(event);
        let stem = format!("{}.{}", c.tag, c.variant);
        assert!(
            id == &stem || id.starts_with(&format!("{stem}.")),
            "sample id `{id}` does not start with `{stem}`"
        );
    }
}

/// The names in the matches are the serde names: the outer key is the
/// family tag and, inside a family, the inner key is the variant name.
#[test]
fn variant_names_match_the_serde_tags() {
    for (id, event) in all_samples() {
        let c = classify(&event);
        let value = serde_json::to_value(&event).expect("serialize sample");
        let outer = value.as_object().expect("externally tagged object");
        assert_eq!(outer.len(), 1, "{id}: one outer tag");
        let inner = outer
            .get(c.tag)
            .unwrap_or_else(|| panic!("{id}: outer tag is not `{}`", c.tag));
        if c.tag == "unreadChanged" {
            continue;
        }
        let inner = inner.as_object().expect("struct variant object");
        assert_eq!(inner.len(), 1, "{id}: one inner tag");
        assert!(
            inner.contains_key(c.variant),
            "{id}: inner tag is not `{}`",
            c.variant
        );
    }
}

#[test]
fn every_key_is_camel_case() {
    let mut bad = Vec::new();
    for (id, value) in serialized_samples() {
        collect_bad_keys(&value, &format!("[{id}] $"), &mut bad);
    }
    assert!(
        bad.is_empty(),
        "keys that are not camelCase:\n{}",
        bad.join("\n")
    );
}

#[test]
fn json_matches_the_golden_file() {
    let actual = serialized_samples();
    let mut rendered =
        serde_json::to_string_pretty(&Value::Object(actual.clone())).expect("render golden");
    rendered.push('\n');
    let path = golden_path();
    if std::env::var_os("UPDATE_GOLDEN").is_some_and(|v| v == "1") {
        std::fs::write(&path, rendered).expect("write golden file");
        return;
    }
    let on_disk = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {}: {e} (run with UPDATE_GOLDEN=1)", path.display()));
    if on_disk == rendered {
        return;
    }
    let golden: Map<String, Value> = serde_json::from_str(&on_disk).expect("golden file is JSON");
    let mut report = Vec::new();
    for (id, value) in &actual {
        match golden.get(id) {
            None => report.push(format!("added:   {id}")),
            Some(old) if old != value => {
                report.push(format!("changed: {id}\n  golden: {old}\n  actual: {value}"));
            }
            Some(_) => {}
        }
    }
    for id in golden.keys().filter(|id| !actual.contains_key(*id)) {
        report.push(format!("removed: {id}"));
    }
    if report.is_empty() {
        report.push("formatting differs (re-run with UPDATE_GOLDEN=1)".to_owned());
    }
    panic!(
        "subscription event JSON differs from {}:\n{}",
        path.display(),
        report.join("\n")
    );
}

#[test]
fn channel_message_family_is_covered() {
    check_family("channelMessage");
}

#[test]
fn typing_family_is_covered() {
    check_family("typing");
}

#[test]
fn presence_family_is_covered() {
    check_family("presence");
}

#[test]
fn membership_family_is_covered() {
    check_family("membership");
}

#[test]
fn friend_family_is_covered() {
    check_family("friend");
}

#[test]
fn crypto_family_is_covered() {
    check_family("crypto");
}

#[test]
fn voice_family_is_covered() {
    check_family("voice");
}

#[test]
fn governance_family_is_covered() {
    check_family("governance");
}

#[test]
fn social_family_is_covered() {
    check_family("social");
}

#[test]
fn call_family_is_covered() {
    check_family("call");
}

#[test]
fn notification_family_is_covered() {
    check_family("notification");
}

#[test]
fn network_family_is_covered() {
    check_family("network");
}

#[test]
fn system_family_is_covered() {
    check_family("system");
}

#[test]
fn unread_changed_is_covered() {
    check_family("unreadChanged");
}
