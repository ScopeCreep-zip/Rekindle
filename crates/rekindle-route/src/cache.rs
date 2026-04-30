//! Peer route cache with staleness tracking.

use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedRoute {
    pub route_blob: Vec<u8>,
    pub updated_at: Instant,
}

impl CachedRoute {
    pub fn new(route_blob: Vec<u8>, updated_at: Instant) -> Self {
        Self {
            route_blob,
            updated_at,
        }
    }

    pub fn is_stale_at(&self, now: Instant, max_age: Duration) -> bool {
        now.saturating_duration_since(self.updated_at) > max_age
    }
}

#[derive(Debug, Clone)]
pub struct RouteCache {
    routes: HashMap<String, CachedRoute>,
    max_age: Duration,
}

impl RouteCache {
    pub fn new(max_age: Duration) -> Self {
        Self {
            routes: HashMap::new(),
            max_age,
        }
    }

    pub fn insert_at(&mut self, peer_id: impl Into<String>, route_blob: Vec<u8>, now: Instant) {
        self.routes
            .insert(peer_id.into(), CachedRoute::new(route_blob, now));
    }

    pub fn get(&self, peer_id: &str) -> Option<&CachedRoute> {
        self.routes.get(peer_id)
    }

    pub fn remove(&mut self, peer_id: &str) -> Option<CachedRoute> {
        self.routes.remove(peer_id)
    }

    pub fn evict_stale_at(&mut self, now: Instant) -> Vec<String> {
        let stale: Vec<String> = self
            .routes
            .iter()
            .filter(|(_, route)| route.is_stale_at(now, self.max_age))
            .map(|(peer_id, _)| peer_id.clone())
            .collect();

        for peer_id in &stale {
            self.routes.remove(peer_id);
        }

        stale
    }

    pub fn len(&self) -> usize {
        self.routes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::RouteCache;

    #[test]
    fn evicts_only_stale_routes() {
        let start = Instant::now();
        let mut cache = RouteCache::new(Duration::from_secs(120));
        cache.insert_at("alice", vec![1], start);
        cache.insert_at("bob", vec![2], start + Duration::from_secs(90));

        let evicted = cache.evict_stale_at(start + Duration::from_secs(121));
        assert_eq!(evicted, vec!["alice".to_string()]);
        assert!(cache.get("alice").is_none());
        assert!(cache.get("bob").is_some());
    }
}
