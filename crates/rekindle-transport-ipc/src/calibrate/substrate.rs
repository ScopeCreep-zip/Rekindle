//! Substrate — the hardware/OS/runtime triple identifying where a
//! measurement was taken.
//!
//! The substrate names the portability class, not the exact machine.
//! Exact machine details (CPU model, cache geometry, kernel version)
//! are carried in the measurement record's provenance, not here.

/// The substrate triple: arch.os.runtime.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Substrate {
    arch: String,
    os: String,
    runtime: String,
}

impl Substrate {
    /// Parse from dotted triple string: "x86_64.linux.uring".
    pub fn parse(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() != 3 {
            return None;
        }
        for part in &parts {
            if part.is_empty() || !part.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
                return None;
            }
        }
        Some(Self {
            arch: parts[0].to_owned(),
            os: parts[1].to_owned(),
            runtime: parts[2].to_owned(),
        })
    }

    /// The canonical dotted string.
    pub fn canonical(&self) -> String {
        format!("{}.{}.{}", self.arch, self.os, self.runtime)
    }

    /// Auto-detect the current substrate from the runtime environment.
    pub fn auto_detect() -> Self {
        Self {
            arch: detect_arch(),
            os: detect_os(),
            runtime: detect_runtime(),
        }
    }

    pub fn arch(&self) -> &str { &self.arch }
    pub fn os(&self) -> &str { &self.os }
    pub fn runtime(&self) -> &str { &self.runtime }
}

fn detect_arch() -> String {
    #[cfg(target_arch = "x86_64")]
    { "x86_64".to_owned() }
    #[cfg(target_arch = "aarch64")]
    { "aarch64".to_owned() }
    #[cfg(target_arch = "riscv64")]
    { "riscv64".to_owned() }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64", target_arch = "riscv64")))]
    { std::env::consts::ARCH.to_owned() }
}

fn detect_os() -> String {
    #[cfg(target_os = "linux")]
    { "linux".to_owned() }
    #[cfg(target_os = "macos")]
    { "macos".to_owned() }
    #[cfg(target_os = "windows")]
    { "windows".to_owned() }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    { std::env::consts::OS.to_owned() }
}

fn detect_runtime() -> String {
    #[cfg(target_os = "linux")]
    { "uring".to_owned() }
    #[cfg(target_os = "macos")]
    { "kqueue".to_owned() }
    #[cfg(target_os = "windows")]
    { "iocp".to_owned() }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    { "bare".to_owned() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid() {
        let s = Substrate::parse("x86_64.linux.uring").unwrap();
        assert_eq!(s.arch(), "x86_64");
        assert_eq!(s.os(), "linux");
        assert_eq!(s.runtime(), "uring");
    }

    #[test]
    fn canonical_roundtrip() {
        let s = Substrate::parse("aarch64.macos.kqueue").unwrap();
        assert_eq!(s.canonical(), "aarch64.macos.kqueue");
        let reparsed = Substrate::parse(&s.canonical()).unwrap();
        assert_eq!(s, reparsed);
    }

    #[test]
    fn rejects_wrong_part_count() {
        assert!(Substrate::parse("x86_64.linux").is_none());
        assert!(Substrate::parse("x86_64.linux.uring.extra").is_none());
        assert!(Substrate::parse("").is_none());
    }

    #[test]
    fn rejects_invalid_chars() {
        assert!(Substrate::parse("X86_64.linux.uring").is_none());
        assert!(Substrate::parse("x86_64.Linux.uring").is_none());
    }

    #[test]
    fn auto_detect_produces_valid() {
        let s = Substrate::auto_detect();
        assert!(!s.arch().is_empty());
        assert!(!s.os().is_empty());
        assert!(!s.runtime().is_empty());
        assert!(Substrate::parse(&s.canonical()).is_some());
    }
}
