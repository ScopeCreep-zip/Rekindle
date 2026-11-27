use super::{names_role_column, scan, scan_sql};

#[test]
fn flags_cache_writes_writers_and_columns() {
    let bad = r#"
            fn f(c: &mut State, m: &mut Membership, conn: &Connection) {
                c.my_role_ids = vec![0];
                m.role_ids.push(5);
                c.member_roles.insert(k, v);
                c.my_role_ids.clone_from(&m.role_ids);
                rekindle_db::repo::members::set_role_ids(conn, a, b, c, &[0], false);
                store.set_member_roles("c", map);
                conn.execute("UPDATE communities SET my_role_ids = ?1", []);
                conn.execute("SELECT role_ids FROM community_members", []);
            }
            async fn apply_role_assignment(&self) {}
        "#;
    let hits = scan("f.rs", bad);
    assert_eq!(hits.len(), 9, "{hits:?}");
    let sql = "CREATE TABLE t (\n    role_ids TEXT, -- cached\n    name TEXT\n);";
    assert_eq!(scan_sql("s.sql", sql), ["s.sql:2  a `role_ids` column"]);
}

#[test]
fn allows_reads_wire_fields_and_derived_names() {
    let ok = r#"
            fn f(c: &State, gov: &GovernanceState) -> View {
                let mine = c.my_role_ids.clone();
                let all = member_role_ids(gov);
                let roles = all.get("p").cloned();
                let payload = ControlPayload::OnboardingComplete { pseudonym_key: p, role_ids: mine };
                let view = View { my_role_ids: c.my_role_ids.clone(), role_ids: roles };
                tracing::info!("member_role_ids computed");
                view
            }
        "#;
    assert!(scan("f.rs", ok).is_empty(), "{:?}", scan("f.rs", ok));
    assert!(!names_role_column("member_role_ids"));
    assert!(!names_role_column("role_ids_of"));
    assert!(names_role_column("SET my_role_ids = ?1"));
    assert!(names_role_column("(a, role_ids, b)"));
}
