//! Leader lease. A takeover bumps the fencing token so a stale leader cannot
//! keep dispatching.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaderLease {
    pub holder: String,
    pub fencing: u64,
    pub until_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claim {
    Held(LeaderLease),
    Rejected { holder: String, fencing: u64 },
}

pub fn claim(current: Option<LeaderLease>, candidate: &str, now_ms: u64, ttl_ms: u64) -> Claim {
    match current {
        Some(lease) if now_ms < lease.until_ms && lease.holder != candidate => Claim::Rejected {
            holder: lease.holder,
            fencing: lease.fencing,
        },
        Some(lease) => {
            let fencing = if lease.holder == candidate {
                lease.fencing
            } else {
                lease.fencing.saturating_add(1)
            };
            Claim::Held(LeaderLease {
                holder: candidate.to_string(),
                fencing,
                until_ms: now_ms.saturating_add(ttl_ms),
            })
        }
        None => Claim::Held(LeaderLease {
            holder: candidate.to_string(),
            fencing: 1,
            until_ms: now_ms.saturating_add(ttl_ms),
        }),
    }
}

pub fn dispatch_allowed(lease: &LeaderLease, holder: &str, fencing: u64, now_ms: u64) -> bool {
    lease.holder == holder && lease.fencing == fencing && now_ms < lease.until_ms
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_live_leader_rejects_a_second_candidate_and_takeover_bumps_fencing() {
        let first = match claim(None, "leader-a", 0, 1_000) {
            Claim::Held(lease) => lease,
            Claim::Rejected { .. } => panic!("first claim should hold"),
        };
        assert_eq!(first.fencing, 1);
        assert!(matches!(
            claim(Some(first.clone()), "leader-b", 500, 1_000),
            Claim::Rejected { fencing: 1, .. }
        ));
        let taken = match claim(Some(first.clone()), "leader-b", 1_000, 1_000) {
            Claim::Held(lease) => lease,
            Claim::Rejected { .. } => panic!("expired lease should be takeable"),
        };
        assert_eq!(taken.fencing, 2);
        assert!(!dispatch_allowed(&taken, "leader-a", 1, 1_100));
        assert!(dispatch_allowed(&taken, "leader-b", 2, 1_100));
    }
}
