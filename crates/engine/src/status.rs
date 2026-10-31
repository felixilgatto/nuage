use nuage_core::{Account, StatusReport};

/// Polls `vfs/stats`, `core/stats` and `operations/about`, aggregates the
/// per-account states and publishes `Event`s.
///
/// **Skeleton**: no polling yet; the report lists no account.
#[derive(Debug, Default)]
pub struct StatusPoller {}

impl StatusPoller {
    /// Current status report.
    pub fn report(&self, _accounts: &[Account]) -> StatusReport {
        StatusReport::empty()
    }
}
