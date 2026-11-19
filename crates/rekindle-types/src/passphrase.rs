//! A passphrase in transit (security-hardening roadmap 1.7, plan E7.1):
//! the requests that carry one (`Unlock`, `IdentityCreate`, the archive
//! requests) hold it here, so the heap copy is zeroized when the request
//! drops instead of staying in freed memory.

use std::fmt;
use std::ops::Deref;

use serde::{Deserialize, Serialize};
use zeroize::{ZeroizeOnDrop, Zeroizing};

/// A passphrase, zeroized on drop; on the wire it is a plain string.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Passphrase(Zeroizing<String>);

impl ZeroizeOnDrop for Passphrase {}

impl Passphrase {
    /// The passphrase text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl Deref for Passphrase {
    type Target = str;

    fn deref(&self) -> &str {
        self.0.as_str()
    }
}

impl From<String> for Passphrase {
    fn from(text: String) -> Self {
        Self(Zeroizing::new(text))
    }
}

impl From<&str> for Passphrase {
    fn from(text: &str) -> Self {
        Self(Zeroizing::new(text.to_owned()))
    }
}

impl fmt::Debug for Passphrase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Passphrase(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_is_a_plain_string_on_the_wire_and_redacted_in_logs() {
        let pass = Passphrase::from("correct horse");
        let json = serde_json::to_string(&pass).unwrap();
        assert_eq!(json, "\"correct horse\"");
        assert_eq!(serde_json::from_str::<Passphrase>(&json).unwrap(), pass);
        assert_eq!(&*pass, "correct horse");
        assert_eq!(format!("{pass:?}"), "Passphrase(<redacted>)");
    }
}
