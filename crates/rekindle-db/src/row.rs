//! Strict row readers (plan D2): a missing column, a wrong type, an enum
//! value outside its set or JSON that does not parse is an error naming the
//! column, never a silent default.

use rusqlite::types::Type;
use rusqlite::Row;

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

fn invalid(row: &Row<'_>, column: &str, detail: &str) -> rusqlite::Error {
    let index = row.as_ref().column_index(column).unwrap_or(0);
    rusqlite::Error::FromSqlConversionFailure(
        index,
        Type::Text,
        format!("column {column}: {detail}").into(),
    )
}

#[cfg(test)]
mod tests {
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

    fn query<T>(
        sql: &str,
        read: impl FnOnce(&Row<'_>) -> rusqlite::Result<T>,
    ) -> rusqlite::Result<T> {
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
}
