//! One row-reading path for every repo (gate hardening item 2, cluster b4):
//! a row type says once how it is read from a SELECT's columns, and
//! [`query_list`], [`query_one`] and [`query_opt`] prepare the query, bind its
//! parameters and map each row with it. The SQL stays at the call site; a
//! malformed row is the reader's typed error, never a default.

use rusqlite::{Connection, OptionalExtension as _, Params, Row};

/// A value read from one result row, its columns in the order its queries
/// select them.
pub trait FromRow: Sized {
    /// The value in `row`.
    ///
    /// # Errors
    /// A column is missing or does not decode as its field.
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self>;
}

/// A single-column row: the column decoded as the type.
macro_rules! scalar_rows {
    ($($t:ty),* $(,)?) => {$(
        impl FromRow for $t {
            fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
                row.get(0)
            }
        }
    )*};
}

scalar_rows!(
    String,
    i64,
    i32,
    u32,
    u16,
    u8,
    bool,
    Vec<u8>,
    Option<String>,
    Option<i64>
);

/// A row read as a tuple: column `i` decoded as element `i`.
macro_rules! tuple_rows {
    ($(($($t:ident $i:tt),+)),* $(,)?) => {$(
        impl<$($t: rusqlite::types::FromSql),+> FromRow for ($($t,)+) {
            fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
                Ok(($(row.get($i)?,)+))
            }
        }
    )*};
}

tuple_rows!(
    (A 0, B 1),
    (A 0, B 1, C 2),
    (A 0, B 1, C 2, D 3),
    (A 0, B 1, C 2, D 3, E 4),
    (A 0, B 1, C 2, D 3, E 4, F 5),
);

/// Every row `sql` returns for `params`, in the query's order.
///
/// # Errors
/// The query failed or a row did not read as `T`.
pub fn query_list<T: FromRow>(
    conn: &Connection,
    sql: &str,
    params: impl Params,
) -> rusqlite::Result<Vec<T>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params, T::from_row)?;
    rows.collect()
}

/// The first row `sql` returns for `params`.
///
/// # Errors
/// No row matched ([`rusqlite::Error::QueryReturnedNoRows`]), the query
/// failed, or the row did not read as `T`.
pub fn query_one<T: FromRow>(
    conn: &Connection,
    sql: &str,
    params: impl Params,
) -> rusqlite::Result<T> {
    conn.query_row(sql, params, T::from_row)
}

/// The first row `sql` returns for `params`, `None` when none matched.
///
/// # Errors
/// The query failed or the row did not read as `T`.
pub fn query_opt<T: FromRow>(
    conn: &Connection,
    sql: &str,
    params: impl Params,
) -> rusqlite::Result<Option<T>> {
    conn.query_row(sql, params, T::from_row).optional()
}

#[cfg(test)]
#[path = "query_tests.rs"]
mod tests;
