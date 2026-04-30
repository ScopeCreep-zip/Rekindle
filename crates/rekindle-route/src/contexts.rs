//! Routing-context selection metadata for chat and voice paths.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteContextKind {
    Safe,
    Voice,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteContextSpec {
    pub kind: RouteContextKind,
    pub hop_count: usize,
    pub sender_anonymous: bool,
    pub ordered: bool,
}

impl RouteContextSpec {
    pub fn rc_safe() -> Self {
        Self {
            kind: RouteContextKind::Safe,
            hop_count: 3,
            sender_anonymous: true,
            ordered: false,
        }
    }

    pub fn rc_voice() -> Self {
        Self {
            kind: RouteContextKind::Voice,
            hop_count: 0,
            sender_anonymous: false,
            ordered: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DualRoutingContexts<T> {
    pub safe: T,
    pub voice: T,
}

impl<T> DualRoutingContexts<T> {
    pub fn new(safe: T, voice: T) -> Self {
        Self { safe, voice }
    }

    pub fn get(&self, kind: RouteContextKind) -> &T {
        match kind {
            RouteContextKind::Safe => &self.safe,
            RouteContextKind::Voice => &self.voice,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DualRoutingContexts, RouteContextKind, RouteContextSpec};

    #[test]
    fn safe_and_voice_specs_match_architecture() {
        let safe = RouteContextSpec::rc_safe();
        assert_eq!(safe.kind, RouteContextKind::Safe);
        assert_eq!(safe.hop_count, 3);
        assert!(safe.sender_anonymous);

        let voice = RouteContextSpec::rc_voice();
        assert_eq!(voice.kind, RouteContextKind::Voice);
        assert_eq!(voice.hop_count, 0);
        assert!(!voice.sender_anonymous);
    }

    #[test]
    fn dual_contexts_select_by_kind() {
        let contexts = DualRoutingContexts::new("safe", "voice");
        assert_eq!(contexts.get(RouteContextKind::Safe), &"safe");
        assert_eq!(contexts.get(RouteContextKind::Voice), &"voice");
    }
}
