//! Strict row readers (plan D2): a missing column, a wrong type, an enum
//! value outside its set or JSON that does not parse is an error naming the
//! column, never a silent default.

use rusqlite::types::Type;
use rusqlite::Row;

mod query;

pub use query::{query_list, query_one, query_opt, FromRow};

/// A column decoded as `T`.
///
/// # Errors
/// The column is missing or holds another type.
pub fn get<T: rusqlite::types::FromSql>(row: &Row<'_>, column: &str) -> rusqlite::Result<T> {
    row.get(column)
}

/// An integer column that holds a count, clock or timestamp: written from
/// values that fit `i64`, so a negative one is a corrupt row.
///
/// # Errors
/// The column at `index` is missing, not an integer, or negative.
pub fn get_u64(row: &Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(index)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}

/// A text column parsed as `T` (an enum stored by its name).
///
/// # Errors
/// The column is missing, not text, or not one of `T`'s values.
pub fn get_enum<T>(row: &Row<'_>, column: &str) -> rusqlite::Result<T>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    let text: String = row.get(column)?;
    text.parse::<T>()
        .map_err(|e| invalid(row, column, &format!("{text:?}: {e}")))
}

/// A text column parsed as JSON into `T`.
///
/// # Errors
/// The column is missing, not text, or not valid JSON for `T`.
pub fn get_json<T: serde::de::DeserializeOwned>(
    row: &Row<'_>,
    column: &str,
) -> rusqlite::Result<T> {
    let text: String = row.get(column)?;
    serde_json::from_str(&text).map_err(|e| invalid(row, column, &e.to_string()))
}

/// A nullable text column parsed as JSON into `T` (`None` when NULL).
///
/// # Errors
/// The column is missing, not text, or not valid JSON for `T`.
pub fn get_json_opt<T: serde::de::DeserializeOwned>(
    row: &Row<'_>,
    column: &str,
) -> rusqlite::Result<Option<T>> {
    let text: Option<String> = row.get(column)?;
    text.map(|text| serde_json::from_str(&text).map_err(|e| invalid(row, column, &e.to_string())))
        .transpose()
}

/// A text column holding a 16-byte role id as 32 hex characters.
///
/// # Errors
/// The column is missing, not text, or not a 16-byte hex id.
pub fn get_role_id(row: &Row<'_>, column: &str) -> rusqlite::Result<rekindle_types::id::RoleId> {
    let text: String = row.get(column)?;
    rekindle_types::id::RoleId::from_hex(&text)
        .ok_or_else(|| invalid(row, column, "not a 16-byte hex role id"))
}

fn invalid(row: &Row<'_>, column: &str, detail: &str) -> rusqlite::Error {
    let index = row.as_ref().column_index(column).unwrap_or(0);
    rusqlite::Error::FromSqlConversionFailure(
        index,
        Type::Text,
        format!("column {column}: {detail}").into(),
    )
}

#[cfg(test)]
#[path = "row_tests.rs"]
mod tests;
