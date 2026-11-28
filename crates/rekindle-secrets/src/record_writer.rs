//! DHT writer keypair text, held in zeroizing storage across clones.

use std::fmt;

use data_encoding::BASE64URL_NOPAD;
use zeroize::Zeroizing;

/// A record owner's credential. Logs can show its public key separately.
#[derive(Clone, PartialEq, Eq)]
pub struct RecordWriterSecret(Zeroizing<String>);

impl RecordWriterSecret {
    /// Wrap keypair text supplied by a vault or a protocol boundary.
    #[must_use]
    pub fn from_text(text: &str) -> Self {
        Self(Zeroizing::new(text.to_owned()))
    }

    /// Encode the secret half directly into its final zeroizing buffer.
    #[must_use]
    pub fn from_parts(public_text: &str, secret: &[u8]) -> Self {
        let capacity = public_text.len() + 1 + BASE64URL_NOPAD.encode_len(secret.len());
        let mut text = Zeroizing::new(String::with_capacity(capacity));
        text.push_str(public_text);
        text.push(':');
        BASE64URL_NOPAD.encode_append(secret, &mut text);
        Self(text)
    }

    /// Borrow the keypair's native spelling at a signing or vault boundary.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Transfer the owner to a caller using the vault's zeroizing type.
    #[must_use]
    pub fn into_zeroizing(self) -> Zeroizing<String> {
        self.0
    }
}

impl fmt::Debug for RecordWriterSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RecordWriterSecret(…)")
    }
}
