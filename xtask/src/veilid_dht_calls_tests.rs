use super::{names_veilid_core, scan};

#[test]
fn flags_veilid_core_in_a_port_crate() {
    let src = "use rekindle_records::port::RecordPort;\nfn f(_: veilid_core::KeyPair) {}\n";
    let hits = names_veilid_core("crates/rekindle-transport/src/x.rs", src);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(hits[0].starts_with("crates/rekindle-transport/src/x.rs:2"));
    assert!(names_veilid_core("crates/rekindle-records/src/y.rs", "fn g() {}").is_empty());
}

#[test]
fn flags_calls_outside_their_owner() {
    let src = r"
            async fn f(rc: veilid_core::RoutingContext, api: veilid_core::VeilidAPI) {
                let _ = rc.get_dht_value(k, 0, false).await;
                let _ = api.import_remote_private_route(blob);
                let _ = api.new_private_route().await;
            }
        ";
    let hits = scan("src-tauri/src/services/x.rs", src);
    assert_eq!(hits.len(), 3, "{hits:?}");
    assert!(hits[0].starts_with("src-tauri/src/services/x.rs:3"));
}

#[test]
fn allows_each_owner_its_own_calls() {
    let pool = "fn f() { rc.set_dht_value(k, 0, v, None); }";
    assert!(scan("crates/rekindle-protocol/src/dht/backend.rs", pool).is_empty());
    assert_eq!(
        scan("crates/rekindle-protocol/src/dht/pool/io.rs", pool).len(),
        1,
        "the pool calls its backend, never Veilid"
    );
    let imports = "fn f() { api.import_remote_private_route(b); }";
    assert!(scan("crates/rekindle-protocol/src/dht/route_imports.rs", imports).is_empty());
    let own = "fn f() { api.release_private_route(id); }";
    assert!(scan("crates/rekindle-protocol/src/own_routes.rs", own).is_empty());
    assert!(scan("src-tauri/src/services/relay/offer.rs", own).is_empty());
}

#[test]
fn an_owner_of_one_rule_is_not_an_owner_of_another() {
    let src = "fn f() { api.release_private_route(id); }";
    assert_eq!(
        scan("crates/rekindle-protocol/src/dht/route_imports.rs", src).len(),
        1,
        "the importer never releases"
    );
}
