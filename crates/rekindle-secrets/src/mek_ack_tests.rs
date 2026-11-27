use super::*;

fn ctx<'a>(transfer_id: &'a [u8], joiner: &'a str) -> MekAckContext<'a> {
    MekAckContext {
        community_id: "c1",
        scope: "",
        generation: 3,
        transfer_id,
        inviter: "aa",
        joiner,
    }
}

#[test]
fn only_the_same_key_and_context_confirm() {
    let mek = [7u8; 32];
    let tag = mek_ack_tag(&mek, &ctx(b"t1", "bb"));
    assert!(verify_mek_ack_tag(&mek, &ctx(b"t1", "bb"), &tag));
    assert!(!verify_mek_ack_tag(&[8u8; 32], &ctx(b"t1", "bb"), &tag));
    assert!(!verify_mek_ack_tag(&mek, &ctx(b"t1", "cc"), &tag));
    assert!(!verify_mek_ack_tag(&mek, &ctx(b"t2", "bb"), &tag));
    let mut other_gen = ctx(b"t1", "bb");
    other_gen.generation = 4;
    assert!(!verify_mek_ack_tag(&mek, &other_gen, &tag));
    assert!(!verify_mek_ack_tag(&mek, &ctx(b"t1", "bb"), &tag[..31]));
}

/// Length prefixes keep a byte from moving between adjacent fields.
#[test]
fn fields_do_not_run_together() {
    let mek = [7u8; 32];
    let a = MekAckContext {
        inviter: "aab",
        joiner: "b",
        ..ctx(b"t", "")
    };
    let b = MekAckContext {
        inviter: "aa",
        joiner: "bb",
        ..ctx(b"t", "")
    };
    assert_ne!(mek_ack_tag(&mek, &a), mek_ack_tag(&mek, &b));
}
