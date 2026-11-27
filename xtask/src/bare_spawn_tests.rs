use super::*;

#[test]
fn flags_bare_blocking_work_in_the_host_only() {
    let src = "fn work() { tokio::task::spawn_blocking(|| ()); std::thread::spawn(|| ()); }";
    let (hits, _) = scan("crates/rekindle-node/src/host/a.rs", src);
    assert_eq!(hits.len(), 2, "{hits:?}");
    assert!(hits[0].contains("run_blocking"), "{hits:?}");
    let (hits, _) = scan("src-tauri/src/a.rs", src);
    assert!(hits.is_empty(), "{hits:?}");
}

#[test]
fn flags_bare_spawns_and_names_the_function() {
    let src = "fn work() { tokio::spawn(async {}); tauri::async_runtime::spawn(async {}); }";
    let (hits, _) = scan("src-tauri/src/x.rs", src);
    assert_eq!(hits.len(), 2, "{hits:?}");
    assert!(hits[0].contains("x.rs:1") && hits[0].contains("`work`"));
}

#[test]
fn ignores_owned_spawns_and_test_code() {
    let src = r"
            fn owned(set: &mut tokio::task::JoinSet<()>) { set.spawn(async {}); }
            #[cfg(test)]
            mod tests { fn t() { tokio::spawn(async {}); } }
            #[tokio::test]
            async fn t2() { tokio::spawn(async {}); }
        ";
    assert!(scan("src-tauri/src/x.rs", src).0.is_empty());
}

#[test]
fn allowlist_is_per_function() {
    let src = "fn start_dispatch_loop() { tauri::async_runtime::spawn(async {}); }
                   fn other() { tauri::async_runtime::spawn(async {}); }";
    let (hits, used) = scan("src-tauri/src/services/veilid/lifecycle/dispatch.rs", src);
    assert_eq!(hits.len(), 1);
    assert_eq!(used.len(), 1);
    assert!(hits[0].contains("`other`"));
}
