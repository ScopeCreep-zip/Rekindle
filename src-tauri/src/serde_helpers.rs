use serde::{self, Deserialize, Deserializer, Serializer};

/// Serialize a `u64` as a JSON string to avoid JavaScript `Number` precision loss.
///
/// JavaScript's `Number` type (IEEE 754 double) only has 53 bits of integer precision.
/// `u64` values above `2^53 - 1` lose their low bits when parsed as JSON numbers,
/// which silently corrupts permission bitfields (e.g. the ADMINISTRATOR flag at bit 3
/// is lost when the Owner role's `Permissions::all()` value exceeds safe integer range).
///
/// The `&u64` reference is required by serde's `serialize_with` contract.
#[allow(clippy::trivially_copy_pass_by_ref)]
pub fn serialize_u64_as_string<S>(value: &u64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&value.to_string())
}

/// Deserialize a `u64` from either a JSON string or number.
///
/// Accepts both `"123"` (string) and `123` (number) for backward compatibility.
pub fn deserialize_u64_from_string_or_number<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrNumber {
        String(String),
        Number(u64),
    }

    match StringOrNumber::deserialize(deserializer)? {
        StringOrNumber::String(s) => s.parse::<u64>().map_err(serde::de::Error::custom),
        StringOrNumber::Number(n) => Ok(n),
    }
}
