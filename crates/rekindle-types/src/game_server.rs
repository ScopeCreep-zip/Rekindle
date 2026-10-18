//! Architecture §27 — community-favourited game-server metadata,
//! broadcast over the `GameServerAdded` control envelope. Wire shape
//! preserved from the pre-migration JSON form.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameServerInfo {
    /// 16-byte UUID hex (`gs_<32 hex>`).
    pub id: String,
    /// Game identifier (rich-presence id), serialized as string for JSON
    /// number-precision safety.
    pub game_id: String,
    pub label: String,
    /// `host:port` string the launcher uses.
    pub address: String,
    /// Hex-encoded pseudonym of the member who added the server.
    pub added_by: String,
    pub created_at: u64,
}

/// Longest DNS name a server address may carry (RFC 1035 §2.3.4).
const MAX_HOST_LEN: usize = 253;
/// Longest DNS label (RFC 1035 §2.3.4).
const MAX_LABEL_LEN: usize = 63;

/// A game server's `host:port`, parsed (plan E1.1): the only form a launcher
/// substitutes into the URI it hands the OS. A peer supplies the address
/// (presence, a community's server list), so anything that could change
/// the URI's meaning — a path, query, fragment, percent escape, whitespace,
/// a `+` argument, quotes or a scheme — is refused.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ServerAddress(String);

/// Why a server address was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ServerAddressError {
    #[error("a server address is host:port")]
    Shape,
    #[error("the host is not an IPv4 address, a bracketed IPv6 address or a DNS name")]
    Host,
    #[error("the port is not 1 to 65535")]
    Port,
}

impl ServerAddress {
    /// Parse `host:port`: an IPv4 literal, a bracketed IPv6 literal or an
    /// LDH DNS name (labels `[A-Za-z0-9-]{1,63}`, at most 253 characters),
    /// and a port from 1 to 65535.
    ///
    /// # Errors
    /// Any other form.
    pub fn parse(text: &str) -> Result<Self, ServerAddressError> {
        let (host, port) = if let Some(rest) = text.strip_prefix('[') {
            let (v6, port) = rest.split_once("]:").ok_or(ServerAddressError::Shape)?;
            v6.parse::<std::net::Ipv6Addr>()
                .map_err(|_| ServerAddressError::Host)?;
            (None, port)
        } else {
            let (host, port) = text.rsplit_once(':').ok_or(ServerAddressError::Shape)?;
            (Some(host), port)
        };
        if let Some(host) = host {
            if host.parse::<std::net::Ipv4Addr>().is_err() && !is_dns_name(host) {
                return Err(ServerAddressError::Host);
            }
        }
        if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
            return Err(ServerAddressError::Port);
        }
        match port.parse::<u16>() {
            Ok(p) if p > 0 => Ok(Self(text.to_owned())),
            _ => Err(ServerAddressError::Port),
        }
    }

    /// The address as `host:port`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An LDH DNS name: dot-separated labels of letters, digits and hyphens.
fn is_dns_name(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= MAX_HOST_LEN
        && host.split('.').all(|label| {
            (1..=MAX_LABEL_LEN).contains(&label.len())
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

impl std::fmt::Display for ServerAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for ServerAddress {
    type Error = ServerAddressError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::parse(&text)
    }
}

impl From<ServerAddress> for String {
    fn from(address: ServerAddress) -> Self {
        address.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_addresses_are_host_and_port_only() {
        for good in [
            "1.2.3.4:27015",
            "[::1]:27015",
            "[2001:db8::7]:16567",
            "play.example.com:25565",
            "localhost:1",
        ] {
            assert_eq!(ServerAddress::parse(good).unwrap().as_str(), good);
        }
        for bad in [
            "1.2.3.4/../run/1",
            "1.2.3.4:27015 +exec x",
            "host?x",
            "steam://connect/1.2.3.4:27015",
            "1.2.3.4:27015/x",
            "1.2.3.4:27015#x",
            "1.2.3.4:%32",
            "1.2.3.4",
            "1.2.3.4:0",
            "1.2.3.4:65536",
            "::1:27015",
            "[::1]",
            "\"host\":1",
            "a..b:1",
            "+connect:1",
            "",
        ] {
            assert!(ServerAddress::parse(bad).is_err(), "{bad} was accepted");
        }
        let long_label = format!("{}.com:1", "a".repeat(64));
        assert!(ServerAddress::parse(&long_label).is_err());
    }

    #[test]
    fn a_server_address_decodes_only_when_valid() {
        assert!(serde_json::from_str::<ServerAddress>("\"1.2.3.4:1\"").is_ok());
        assert!(serde_json::from_str::<ServerAddress>("\"1.2.3.4:1 x\"").is_err());
    }

    #[test]
    fn game_server_info_roundtrip() {
        let g = GameServerInfo {
            id: "gs_01".into(),
            game_id: "halo".into(),
            label: "Friday CTF".into(),
            address: "10.0.0.1:27015".into(),
            added_by: "abcd".into(),
            created_at: 42,
        };
        let json = serde_json::to_string(&g).unwrap();
        let back: GameServerInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(g, back);
    }
}
