use super::*;

#[derive(Debug, PartialEq)]
enum Colour {
    Red,
}

impl std::str::FromStr for Colour {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        (s == "red")
            .then_some(Self::Red)
            .ok_or_else(|| format!("unknown colour {s}"))
    }
}

fn query<T>(sql: &str, read: impl FnOnce(&Row<'_>) -> rusqlite::Result<T>) -> rusqlite::Result<T> {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.query_row(sql, [], read)
}

#[test]
fn a_corrupt_value_is_an_error_naming_its_column() {
    assert_eq!(
        query("SELECT 'red' AS c", |r| get_enum::<Colour>(r, "c")).unwrap(),
        Colour::Red
    );
    let err = query("SELECT 'blue' AS c", |r| get_enum::<Colour>(r, "c")).unwrap_err();
    assert!(err.to_string().contains("column c"), "{err}");
    assert!(query("SELECT '[1,' AS j", |r| get_json::<Vec<u8>>(r, "j")).is_err());
    assert_eq!(
        query("SELECT NULL AS j", |r| get_json_opt::<Vec<u8>>(r, "j")).unwrap(),
        None
    );
    assert!(query("SELECT 'x' AS n", |r| get::<i64>(r, "n")).is_err());
    assert!(query("SELECT 1 AS n", |r| get::<i64>(r, "missing")).is_err());
}
