use super::*;

fn role() -> RoleDisplay {
    RoleDisplay {
        id: "03".repeat(16),
        name: "Mod".into(),
        color: 0x00_FF_00,
        tag_colours: crate::dto::community::RoleTagColours {
            dark: 0x00_FF_00,
            light: 0x00_6B_00,
        },
        permissions: crate::permissions::ALL,
        position: 1,
        position_label: "1".into(),
        hoist: true,
        mentionable: false,
        self_assignable: false,
        exclusion_group: Some("rank".into()),
    }
}

#[test]
fn every_variant_names_its_community() {
    assert_eq!(
        GovernanceEvent::RolesChanged {
            community: "c".into(),
            roles: vec![role()],
        }
        .community(),
        "c"
    );
    assert_eq!(
        GovernanceEvent::GovernanceRebuilt {
            community: "c".into()
        }
        .community(),
        "c"
    );
    assert_eq!(
        GovernanceEvent::SegmentsChanged {
            community: "c".into(),
            segments: Vec::new(),
        }
        .community(),
        "c"
    );
}

/// The daemon IPC is postcard: `flatten` and `tag = "..."` fail
/// there at runtime, so only a real round trip catches them. The
/// payloads make this more than a formality — `RoleDisplay` and
/// `ChannelOverviewDisplay` now ride inside these events.
#[test]
fn postcard_round_trips_payload_variants() {
    let events = vec![
        GovernanceEvent::MetadataChanged {
            community: "c".into(),
            name: Some("New".into()),
            description: None,
            icon_hash: None,
            banner_hash: Some("bh".into()),
        },
        GovernanceEvent::RolesChanged {
            community: "c".into(),
            roles: vec![role()],
        },
        GovernanceEvent::ChannelsChanged {
            community: "c".into(),
            channels: vec![ChannelOverviewDisplay {
                id: "ch".into(),
                name: "general".into(),
                kind: "text".into(),
                category_id: None,
                topic: String::new(),
                mek_generation: 4,
                log_key: None,
                sort_order: 0,
                slowmode_seconds: Some(30),
            }],
            categories: vec![CategoryDisplay {
                id: "cat".into(),
                name: "Text".into(),
                sort_order: 0,
            }],
        },
        GovernanceEvent::InviteCreated {
            community: "c".into(),
            invite_id: "0a".repeat(16),
            code_hash: "hash".into(),
            created_by: "pk".into(),
            max_uses: 5,
            uses: 0,
            expires_at: None,
            created_at: 1,
        },
        GovernanceEvent::SegmentsChanged {
            community: "c".into(),
            segments: vec![crate::presence::SegmentDescriptor {
                segment_index: 1,
                registry_key: "reg1".into(),
                anchor: "gov1".into(),
                slot_range_start: 255,
                slot_range_end: 510,
            }],
        },
        GovernanceEvent::KeyStatus {
            community: "c".into(),
            status: crate::dto::community::KeyStatus {
                state: crate::dto::community::KeyState::Pending,
                have: Some(2),
                need: Some(3),
                since_ms: Some(1_700),
            },
        },
    ];
    for event in events {
        let bytes = postcard::to_allocvec(&event).expect("postcard encode");
        let back: GovernanceEvent = postcard::from_bytes(&bytes).expect("postcard decode");
        assert_eq!(format!("{event:?}"), format!("{back:?}"));
    }
}

/// The key status's JSON shape, as the webview reads it (KP).
#[test]
fn key_status_json_shape() {
    let event = GovernanceEvent::KeyStatus {
        community: "c".into(),
        status: crate::dto::community::KeyStatus {
            state: crate::dto::community::KeyState::Pending,
            have: None,
            need: Some(3),
            since_ms: Some(1_700),
        },
    };
    assert_eq!(
        serde_json::to_value(&event).unwrap(),
        serde_json::json!({ "keyStatus": { "community": "c", "status": {
            "state": "pending", "have": null, "need": 3, "sinceMs": 1_700,
        }}})
    );
    assert_eq!(
        serde_json::to_value(crate::dto::community::KeyStatus::default()).unwrap(),
        serde_json::json!({ "state": "current", "have": null, "need": null, "sinceMs": null })
    );
}

/// `permissions::ALL` must stay inside JavaScript's safe-integer
/// range while `RoleDisplay.permissions` is a plain `u64` — see the
/// note on that field. Adding permission bit 53 breaks this.
#[test]
fn permissions_all_survives_a_json_round_trip() {
    let json = serde_json::to_string(&role()).unwrap();
    let back: RoleDisplay = serde_json::from_str(&json).unwrap();
    assert_eq!(
        back.permissions,
        crate::permissions::ALL,
        "ALL exceeded 2^53 — RoleDisplay.permissions needs a string encoding"
    );
    let js_max_safe_integer = (1u64 << 53) - 1;
    assert!(
        back.permissions <= js_max_safe_integer,
        "permission bits grew past JavaScript's safe integer range"
    );
}
