//! Runtime platform detection — CPU model, cache geometry, kernel version.
//!
//! These are provenance fields for the measurement record, NOT the
//! identifier. The identifier carries the substrate (portability class);
//! the record carries the exact machine for attribution and analysis.

/// Provenance details auto-detected from the running platform.
#[derive(Debug, Clone)]
pub struct PlatformInfo {
    pub cpu_model: String,
    pub cpu_microarch: String,
    pub cache_geometry: String,
    pub kernel: String,
    pub runtime_detail: String,
}

impl PlatformInfo {
    /// Auto-detect all platform info from the runtime environment.
    pub fn detect() -> Self {
        Self {
            cpu_model: detect_cpu_model(),
            cpu_microarch: detect_cpu_microarch(),
            cache_geometry: detect_cache_geometry(),
            kernel: detect_kernel(),
            runtime_detail: detect_runtime_detail(),
        }
    }
}

fn detect_cpu_model() -> String {
    #[cfg(target_os = "linux")]
    {
        if let Ok(cpuinfo) = std::fs::read_to_string("/proc/cpuinfo") {
            for line in cpuinfo.lines() {
                if line.starts_with("model name") {
                    if let Some(val) = line.split(':').nth(1) {
                        return val.trim().to_owned();
                    }
                }
            }
        }
    }
    "unknown".to_owned()
}

fn detect_cpu_microarch() -> String {
    #[cfg(target_os = "linux")]
    {
        if let Ok(cpuinfo) = std::fs::read_to_string("/proc/cpuinfo") {
            for line in cpuinfo.lines() {
                if line.starts_with("cpu family") {
                    if let Some(val) = line.split(':').nth(1) {
                        return val.trim().to_owned();
                    }
                }
            }
        }
    }
    "unknown".to_owned()
}

fn detect_cache_geometry() -> String {
    #[cfg(target_os = "linux")]
    {
        let mut parts = Vec::new();
        for index in 0..4u32 {
            let base = format!("/sys/devices/system/cpu/cpu0/cache/index{index}");
            let level = std::fs::read_to_string(format!("{base}/level")).ok();
            let ctype = std::fs::read_to_string(format!("{base}/type")).ok();
            let size = std::fs::read_to_string(format!("{base}/size")).ok();
            if let (Some(l), Some(t), Some(s)) = (level, ctype, size) {
                let l = l.trim();
                let t = t.trim().chars().next().unwrap_or('?');
                let s = s.trim().to_ascii_lowercase();
                // t: D=Data, I=Instruction, U=Unified
                let label = match t {
                    'D' | 'd' => format!("L{l}d"),
                    'I' | 'i' => format!("L{l}i"),
                    _ => format!("L{l}"),
                };
                parts.push(format!("{label}={s}"));
            }
        }
        if !parts.is_empty() {
            return parts.join(",");
        }
    }
    "unknown".to_owned()
}

fn detect_kernel() -> String {
    #[cfg(target_os = "linux")]
    {
        if let Ok(ver) = std::fs::read_to_string("/proc/version") {
            if let Some(first) = ver.split_whitespace().nth(2) {
                return format!("linux-{first}");
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        return format!("macos-{}", std::env::consts::OS);
    }
    "unknown".to_owned()
}

fn detect_runtime_detail() -> String {
    #[cfg(target_os = "linux")]
    {
        // Check io_uring support level from kernel version
        if let Ok(ver) = std::fs::read_to_string("/proc/version") {
            if let Some(kver) = ver.split_whitespace().nth(2) {
                return format!("io_uring kernel={kver}");
            }
        }
    }
    "unknown".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_produces_non_empty() {
        let info = PlatformInfo::detect();
        assert!(!info.cpu_model.is_empty());
        assert!(!info.kernel.is_empty());
    }
}
