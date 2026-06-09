use rekindle_transport_ipc::v3::codec::channel::revoke as revoke_codec;
use rekindle_transport_ipc::v3::dispatch::test_helpers::{make_test_context, assert_no_router_deliveries};
use rekindle_transport_ipc::v3::handlers::channel::revoke;

fn uuid_as_artifact_id(id: uuid::Uuid) -> [u8; 32] {
    let mut artifact_id = [0u8; 32];
    artifact_id[..16].copy_from_slice(id.as_bytes());
    artifact_id
}

#[test]
fn revoke_subscription_removes_it() {
    let (mut ctx, router) = make_test_context();
    let sub_id = uuid::Uuid::from_u128(100);
    ctx.register_subscription(sub_id, &[[0x11; 32]], &[]);
    assert_eq!(ctx.subscription_count(), 1);

    let payload = revoke_codec::encode(&revoke_codec::RevokePayload {
        artifact_kind: 0x02, reason_code: 0, artifact_id: uuid_as_artifact_id(sub_id), revoke_generation: 0,
    });
    revoke::handle(&mut ctx, &payload).unwrap();
    assert_eq!(ctx.subscription_count(), 0);
    assert_no_router_deliveries(&router);
}

#[test]
fn revoke_transfer_id_removes_from_resume_registry() {
    let (mut ctx, router) = make_test_context();
    let tid = uuid::Uuid::from_u128(200);
    ctx.register_resume_state(tid, 0, 0, [0; 32], [0; 32]);
    assert!(ctx.has_resume_state(tid));

    let payload = revoke_codec::encode(&revoke_codec::RevokePayload {
        artifact_kind: 0x03, reason_code: 0, artifact_id: uuid_as_artifact_id(tid), revoke_generation: 0,
    });
    revoke::handle(&mut ctx, &payload).unwrap();
    assert!(!ctx.has_resume_state(tid));
    assert_no_router_deliveries(&router);
}

#[test]
fn revoke_content_hash_removes_from_caches() {
    let (mut ctx, router) = make_test_context();
    let hash = [0x99; 32];
    ctx.sender_cache_mut().store(hash, 1024, 1);
    assert!(ctx.sender_cache_mut().lookup(&hash).is_some());

    let payload = revoke_codec::encode(&revoke_codec::RevokePayload {
        artifact_kind: 0x04, reason_code: 0, artifact_id: hash, revoke_generation: 0,
    });
    revoke::handle(&mut ctx, &payload).unwrap();
    assert!(ctx.sender_cache_mut().lookup(&hash).is_none());
    assert_no_router_deliveries(&router);
}

#[test]
fn revoke_unknown_artifact_kind_is_noop() {
    let (mut ctx, router) = make_test_context();
    let payload = revoke_codec::encode(&revoke_codec::RevokePayload {
        artifact_kind: 0xFF, reason_code: 0, artifact_id: [0; 32], revoke_generation: 0,
    });
    let result = revoke::handle(&mut ctx, &payload);
    assert!(result.is_ok());
    assert_no_router_deliveries(&router);
}
