/// Token cap for one agent. The daemon reserves an estimate before a provider call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenBudget {
    pub cap: u64,
    pub spent: u64,
}

impl TokenBudget {
    pub fn remaining(self) -> u64 {
        self.cap.saturating_sub(self.spent)
    }

    pub fn can_reserve(self, estimate: u64) -> bool {
        estimate <= self.remaining()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_thousand_token_cap_halts_past_the_cap() {
        let budget = TokenBudget {
            cap: 1_000,
            spent: 0,
        };
        assert!(budget.can_reserve(1_000));
        assert!(!budget.can_reserve(1_001));

        let exhausted = TokenBudget {
            cap: 1_000,
            spent: 1_000,
        };
        assert_eq!(exhausted.remaining(), 0);
        assert!(!exhausted.can_reserve(1));
    }
}
