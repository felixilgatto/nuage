use serde::{Deserialize, Serialize};

/// Storage quota of an account, in bytes.
///
/// Every field is optional: not all backends report all values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quota {
    pub total: Option<u64>,
    pub used: Option<u64>,
    pub free: Option<u64>,
    pub trashed: Option<u64>,
}

impl Quota {
    /// Used space as a percentage of the total, when both are known.
    pub fn used_percent(&self) -> Option<f64> {
        match (self.used, self.total) {
            (Some(used), Some(total)) if total > 0 => Some(used as f64 * 100.0 / total as f64),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn used_percent() {
        let q = Quota {
            total: Some(200),
            used: Some(50),
            ..Quota::default()
        };
        assert_eq!(q.used_percent(), Some(25.0));
        assert_eq!(Quota::default().used_percent(), None);
    }
}
