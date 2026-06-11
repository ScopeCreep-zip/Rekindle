//! KindDescriptor — self-asserted descriptor of what a peer is.
//!
//! Presentation metadata with exactly the trust standing of a
//! `DisplayName`: a signed claim. The protocol MUST NOT branch on it.
//! This module is import-forbidden from root/, grant/, trust/,
//! session/, and vault_label/ — enforced by a source-scan CI test.
//!
//! # D-01 / D-13 enforcement
//!
//! One kind of peer. Horizontal equivalence is absolute. Authority
//! differences are expressed through `DelegationGrant` edges and
//! governance policy, never through participant kind. This enum
//! exists solely for UI rendering (icon, label, tooltip) and
//! analytics (what fraction of community members are model-driven).

/// Self-asserted descriptor of what a peer is.
///
/// NOT consumed by any verification, authorization, derivation,
/// or storage-keying code path. Module visibility enforced: this
/// type cannot be imported in enforcement modules.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum KindDescriptor {
    /// Human operator (interactive).
    Human,
    /// Model-driven agent (LLM, generative).
    ModelDriven { model_family: String },
    /// Service (daemon, background process).
    Service { unit_name: String },
    /// Automation (CI/CD, bot, script).
    Automation { description: String },
}

impl KindDescriptor {
    /// Human-readable label for UI rendering.
    pub fn display_label(&self) -> &str {
        match self {
            Self::Human => "Human",
            Self::ModelDriven { .. } => "AI Agent",
            Self::Service { .. } => "Service",
            Self::Automation { .. } => "Bot",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_labels() {
        assert_eq!(KindDescriptor::Human.display_label(), "Human");
        assert_eq!(
            KindDescriptor::ModelDriven { model_family: "claude".into() }.display_label(),
            "AI Agent"
        );
        assert_eq!(
            KindDescriptor::Service { unit_name: "rekindle-node".into() }.display_label(),
            "Service"
        );
        assert_eq!(
            KindDescriptor::Automation { description: "CI runner".into() }.display_label(),
            "Bot"
        );
    }

    #[test]
    fn serde_roundtrip() {
        let kinds = [
            KindDescriptor::Human,
            KindDescriptor::ModelDriven { model_family: "opus".into() },
            KindDescriptor::Service { unit_name: "daemon".into() },
            KindDescriptor::Automation { description: "test".into() },
        ];
        for kind in &kinds {
            let json = serde_json::to_string(kind).unwrap();
            let restored: KindDescriptor = serde_json::from_str(&json).unwrap();
            assert_eq!(kind, &restored);
        }
    }
}
