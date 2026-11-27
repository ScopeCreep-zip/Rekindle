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
#[path = "passphrase_tests.rs"]
mod tests;
