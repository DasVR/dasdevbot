use std::cmp::Ordering;

/// A hybrid-logical-clock timestamp: wall millis, a logical counter, and a node id.
/// Ordering is `(millis, counter, node)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HlcTimestamp {
    pub millis: u64,
    pub counter: u32,
    pub node: String,
}

impl PartialOrd for HlcTimestamp {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HlcTimestamp {
    fn cmp(&self, other: &Self) -> Ordering {
        self.millis
            .cmp(&other.millis)
            .then(self.counter.cmp(&other.counter))
            .then(self.node.cmp(&other.node))
    }
}

impl std::fmt::Display for HlcTimestamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}:{}", self.millis, self.counter, self.node)
    }
}

/// Local HLC. `tick` is a send/local event. `observe` merges a remote stamp.
#[derive(Debug, Clone)]
pub struct HybridClock {
    millis: u64,
    counter: u32,
    node: String,
}

impl HybridClock {
    pub fn new(node: impl Into<String>) -> Self {
        Self {
            millis: 0,
            counter: 0,
            node: node.into(),
        }
    }

    /// Continue a clock from the latest persisted stamp so a restart cannot go backwards.
    pub fn resume(node: impl Into<String>, millis: u64, counter: u32) -> Self {
        Self {
            millis,
            counter,
            node: node.into(),
        }
    }

    pub fn node(&self) -> &str {
        &self.node
    }

    pub fn tick(&mut self, wall_ms: u64) -> HlcTimestamp {
        if wall_ms > self.millis {
            self.millis = wall_ms;
            self.counter = 0;
        } else {
            self.counter = self.counter.saturating_add(1);
        }
        self.stamp()
    }

    pub fn observe(&mut self, wall_ms: u64, remote: &HlcTimestamp) -> HlcTimestamp {
        let next_millis = self.millis.max(remote.millis).max(wall_ms);
        let next_counter = if next_millis == self.millis && next_millis == remote.millis {
            self.counter.max(remote.counter).saturating_add(1)
        } else if next_millis == self.millis {
            self.counter.saturating_add(1)
        } else if next_millis == remote.millis {
            remote.counter.saturating_add(1)
        } else {
            0
        };
        self.millis = next_millis;
        self.counter = next_counter;
        self.stamp()
    }

    fn stamp(&self) -> HlcTimestamp {
        HlcTimestamp {
            millis: self.millis,
            counter: self.counter,
            node: self.node.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_resets_counter_when_wall_moves_forward() {
        let mut clock = HybridClock::new("n1");
        let a = clock.tick(1_000);
        let b = clock.tick(1_000);
        let c = clock.tick(1_500);
        assert_eq!(a.counter, 0);
        assert_eq!(b.millis, 1_000);
        assert_eq!(b.counter, 1);
        assert!(b > a);
        assert_eq!(c.millis, 1_500);
        assert_eq!(c.counter, 0);
        assert!(c > b);
    }

    #[test]
    fn observe_merges_a_remote_stamp() {
        let mut clock = HybridClock::new("device");
        let _ = clock.tick(100);
        let remote = HlcTimestamp {
            millis: 500,
            counter: 4,
            node: "server".into(),
        };
        let merged = clock.observe(100, &remote);
        assert_eq!(merged.millis, 500);
        assert_eq!(merged.counter, 5);
        assert_eq!(merged.node, "device");

        let again = clock.observe(500, &remote);
        assert_eq!(again.millis, 500);
        assert!(again.counter > merged.counter);
    }

    #[test]
    fn resume_does_not_move_backwards() {
        let mut clock = HybridClock::resume("n1", 5_000, 3);
        let next = clock.tick(4_000);
        assert_eq!(next.millis, 5_000);
        assert_eq!(next.counter, 4);
    }
}
