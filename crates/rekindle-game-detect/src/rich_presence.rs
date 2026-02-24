//! Game-specific rich presence data.
//!
//! Provides additional context beyond "playing X" — like server info,
//! map name, game mode, etc. for supported games.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RichPresence {
    pub game_id: u32,
    pub details: Option<String>,
    pub state: Option<String>,
    pub server_ip: Option<String>,
    pub server_port: Option<u16>,
    pub map_name: Option<String>,
    pub player_count: Option<u32>,
    pub max_players: Option<u32>,
}

impl RichPresence {
    /// Create a minimal rich presence with just the game ID.
    pub fn basic(game_id: u32) -> Self {
        Self {
            game_id,
            ..Default::default()
        }
    }

    /// Create rich presence with server info (for multiplayer games).
    pub fn with_server(game_id: u32, server_ip: String, server_port: u16) -> Self {
        Self {
            game_id,
            server_ip: Some(server_ip),
            server_port: Some(server_port),
            ..Default::default()
        }
    }

    /// Return the combined server address as `"ip:port"`, if both are present.
    pub fn server_address(&self) -> Option<String> {
        match (&self.server_ip, self.server_port) {
            (Some(ip), Some(port)) => Some(format!("{ip}:{port}")),
            (Some(ip), None) => Some(ip.clone()),
            _ => None,
        }
    }

    /// Serialize to JSON bytes for DHT publication.
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    /// Deserialize from JSON bytes (from DHT).
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        serde_json::from_slice(data).ok()
    }
}

/// Parse server connection info from game process command-line arguments.
///
/// Recognises common patterns used by Source engine, Quake-derived engines,
/// and other popular multiplayer games:
///   `+connect ip:port`, `-connect ip:port`, `--server ip:port`
///
/// Returns `(ip, port)` if a valid server address is found.
pub fn parse_connect_args(args: &[String]) -> Option<(String, u16)> {
    let connect_flags = ["+connect", "-connect", "--server", "--connect"];

    for window in args.windows(2) {
        let flag = window[0].as_str();
        let value = &window[1];

        if connect_flags.contains(&flag) {
            return parse_host_port(value);
        }
    }

    // Also check single-arg forms: `+connect=ip:port`
    for arg in args {
        for flag in &connect_flags {
            if let Some(value) = arg.strip_prefix(flag).and_then(|s| s.strip_prefix('=')) {
                return parse_host_port(value);
            }
        }
    }

    None
}

/// Parse an `"ip:port"` or `"ip"` string into `(ip, port)`.
fn parse_host_port(s: &str) -> Option<(String, u16)> {
    if let Some((ip, port_str)) = s.rsplit_once(':') {
        let port = port_str.parse::<u16>().ok()?;
        if ip.is_empty() {
            return None;
        }
        Some((ip.to_string(), port))
    } else {
        // Bare IP/hostname without port — use default game port (27015 for Source)
        if s.is_empty() {
            return None;
        }
        Some((s.to_string(), 27015))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_plus_connect() {
        let args: Vec<String> = vec![
            "cs2".into(),
            "+connect".into(),
            "192.168.1.1:27015".into(),
        ];
        let result = parse_connect_args(&args);
        assert_eq!(result, Some(("192.168.1.1".into(), 27015)));
    }

    #[test]
    fn parse_dash_connect() {
        let args: Vec<String> = vec![
            "game.exe".into(),
            "-connect".into(),
            "10.0.0.1:27960".into(),
        ];
        let result = parse_connect_args(&args);
        assert_eq!(result, Some(("10.0.0.1".into(), 27960)));
    }

    #[test]
    fn parse_equals_form() {
        let args: Vec<String> = vec!["game.exe".into(), "+connect=1.2.3.4:28000".into()];
        let result = parse_connect_args(&args);
        assert_eq!(result, Some(("1.2.3.4".into(), 28000)));
    }

    #[test]
    fn parse_no_match() {
        let args: Vec<String> = vec!["game.exe".into(), "--fullscreen".into()];
        let result = parse_connect_args(&args);
        assert_eq!(result, None);
    }

    #[test]
    fn parse_bare_ip_defaults_port() {
        let args: Vec<String> = vec![
            "game.exe".into(),
            "+connect".into(),
            "192.168.1.1".into(),
        ];
        let result = parse_connect_args(&args);
        assert_eq!(result, Some(("192.168.1.1".into(), 27015)));
    }
}
