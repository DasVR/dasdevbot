/// A job lease. The daemon stores these columns; expiry is this comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lease {
    pub owner: String,
    pub until_ms: u64,
}

pub fn is_expired(lease: &Lease, now_ms: u64) -> bool {
    now_ms >= lease.until_ms
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expiry_is_inclusive_at_the_deadline() {
        let lease = Lease {
            owner: "a".into(),
            until_ms: 1_000,
        };
        assert!(!is_expired(&lease, 999));
        assert!(is_expired(&lease, 1_000));
    }
}
