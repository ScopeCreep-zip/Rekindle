//! Argument parsing helpers — durations, permissions, colors, timestamps.

/// Parse a duration string with suffix: "30s", "5m", "1h", "24h", "7d", "2w"
pub fn parse_duration_secs(s: &str) -> anyhow::Result<u64> {
    if let Some(n) = s.strip_suffix('s') { return n.parse().map_err(Into::into); }
    if let Some(n) = s.strip_suffix('m') { return Ok(n.parse::<u64>()? * 60); }
    if let Some(n) = s.strip_suffix('h') { return Ok(n.parse::<u64>()? * 3600); }
    if let Some(n) = s.strip_suffix('d') { return Ok(n.parse::<u64>()? * 86400); }
    if let Some(n) = s.strip_suffix('w') { return Ok(n.parse::<u64>()? * 604_800); }
    s.parse().map_err(|_| anyhow::anyhow!("invalid duration: {s} (use 30s, 5m, 1h, 24h, 7d)"))
}

/// Parse a permission bitmask: decimal or hex with 0x prefix.
pub fn parse_permissions(s: &str) -> anyhow::Result<u64> {
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).map_err(|_| anyhow::anyhow!("invalid hex permissions: {s}"))
    } else {
        s.parse().map_err(|_| anyhow::anyhow!("invalid permissions: {s}"))
    }
}

/// Parse a color hex string: "#FF5733" or "FF5733"
pub fn parse_color(s: &str) -> anyhow::Result<u32> {
    let hex = s.strip_prefix('#').unwrap_or(s);
    u32::from_str_radix(hex, 16).map_err(|_| anyhow::anyhow!("invalid color hex: {s}"))
}

/// Parse a u32 from string.
pub fn parse_u32(s: &str) -> anyhow::Result<u32> {
    s.parse().map_err(|_| anyhow::anyhow!("invalid number: {s}"))
}

/// Parse a --since value: epoch ms or ISO 8601 date (YYYY-MM-DD).
/// Uses chrono for correct date arithmetic (leap years, variable month lengths).
pub fn parse_since_timestamp(s: &str) -> anyhow::Result<u64> {
    if let Ok(ms) = s.parse::<u64>() {
        return Ok(ms);
    }
    // Try YYYY-MM-DD via chrono for correct calendar arithmetic
    if let Ok(date) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let datetime = date.and_hms_opt(0, 0, 0)
            .ok_or_else(|| anyhow::anyhow!("invalid date: {s}"))?;
        #[allow(clippy::cast_sign_loss)]
        let ms = datetime.and_utc().timestamp_millis() as u64;
        return Ok(ms);
    }
    anyhow::bail!("--since: expected epoch ms or YYYY-MM-DD, got '{s}'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_suffixes() {
        assert_eq!(parse_duration_secs("30s").unwrap(), 30);
        assert_eq!(parse_duration_secs("5m").unwrap(), 300);
        assert_eq!(parse_duration_secs("1h").unwrap(), 3600);
        assert_eq!(parse_duration_secs("7d").unwrap(), 604800);
        assert_eq!(parse_duration_secs("2w").unwrap(), 1_209_600);
    }

    #[test]
    fn permissions_decimal_and_hex() {
        assert_eq!(parse_permissions("255").unwrap(), 255);
        assert_eq!(parse_permissions("0xFF").unwrap(), 255);
        assert_eq!(parse_permissions("0XFF").unwrap(), 255);
    }

    #[test]
    fn color_with_and_without_hash() {
        assert_eq!(parse_color("#FF5733").unwrap(), 0xFF5733);
        assert_eq!(parse_color("FF5733").unwrap(), 0xFF5733);
    }

    #[test]
    fn since_epoch_ms() {
        assert_eq!(parse_since_timestamp("1715000000000").unwrap(), 1_715_000_000_000);
    }
}
