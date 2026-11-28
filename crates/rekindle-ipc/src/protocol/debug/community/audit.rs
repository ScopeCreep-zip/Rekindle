//! The audit-log and notification-default requests' `Debug` (plan E3.1 F4).

use crate::protocol::AuditRequest;

impl std::fmt::Debug for AuditRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AuditLog {
                community,
                before_timestamp,
                limit,
            } => f
                .debug_struct("AuditLog")
                .field("community", community)
                .field("before_timestamp", before_timestamp)
                .field("limit", limit)
                .finish(),
            Self::NotificationDefaultGet { community } => f
                .debug_struct("NotificationDefaultGet")
                .field("community", community)
                .finish(),
            Self::NotificationDefaultSet { community, level } => f
                .debug_struct("NotificationDefaultSet")
                .field("community", community)
                .field("level", level)
                .finish(),
        }
    }
}
