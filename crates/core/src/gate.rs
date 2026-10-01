/// Effect classes from the design. Color in the client maps onto these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectClass {
    Read,
    WriteLocal,
    External,
    Destructive,
}

impl EffectClass {
    pub fn as_str(self) -> &'static str {
        match self {
            EffectClass::Read => "read",
            EffectClass::WriteLocal => "write_local",
            EffectClass::External => "external",
            EffectClass::Destructive => "destructive",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "read" => Some(EffectClass::Read),
            "write_local" => Some(EffectClass::WriteLocal),
            "external" => Some(EffectClass::External),
            "destructive" => Some(EffectClass::Destructive),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateOutcome {
    Allow,
    Ask,
    Deny,
}

/// Which classes may proceed without a human. Phase 0 allows reads only.
#[derive(Debug, Clone, Copy)]
pub struct Policy {
    pub auto_approve_read: bool,
    pub auto_approve_write_local: bool,
    pub auto_approve_external: bool,
    pub auto_approve_destructive: bool,
    /// Phase 1 denies destructive outright. Phase 0 leaves this off.
    pub deny_destructive: bool,
    /// Phase 1 denies the external tier until Windows Hello is the production verifier.
    pub deny_external: bool,
}

impl Policy {
    pub fn phase0() -> Self {
        Self {
            auto_approve_read: true,
            auto_approve_write_local: false,
            auto_approve_external: false,
            auto_approve_destructive: false,
            deny_destructive: false,
            deny_external: false,
        }
    }

    /// Phase 1 auto-approves nothing above read.
    /// Destructive is never asked. External is denied outright, whatever
    /// signature or verifier is present. It is re-enabled only after H1
    /// (RSA Hello verify), H2 (key pinning) and the hardware Hello test in
    /// docs/hello-hardware-test.md pass. See [`EXTERNAL_TIER_ENABLED`].
    pub fn phase1() -> Self {
        Self {
            auto_approve_read: true,
            auto_approve_write_local: false,
            auto_approve_external: false,
            auto_approve_destructive: false,
            deny_destructive: true,
            deny_external: true,
        }
    }

    fn auto_approves(self, class: EffectClass) -> bool {
        match class {
            EffectClass::Read => self.auto_approve_read,
            EffectClass::WriteLocal => self.auto_approve_write_local,
            EffectClass::External => self.auto_approve_external,
            EffectClass::Destructive => self.auto_approve_destructive,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct GateInput {
    pub grant: bool,
    pub class: EffectClass,
    pub budget_remaining: bool,
    pub tainted: bool,
}

/// Phase 1 hard switch. While false, the gate and [`crate::authorize_decision`]
/// deny the external tier for every policy, signature and verifier.
pub const EXTERNAL_TIER_ENABLED: bool = false;

/// One gate for every effect. A tainted turn cannot auto-approve external or destructive work.
pub fn decide(input: GateInput, policy: Policy) -> GateOutcome {
    let external_off = policy.deny_external || !EXTERNAL_TIER_ENABLED;
    if external_off && matches!(input.class, EffectClass::External) {
        return GateOutcome::Deny;
    }
    if policy.deny_destructive && matches!(input.class, EffectClass::Destructive) {
        return GateOutcome::Deny;
    }
    if !input.grant || !input.budget_remaining {
        return GateOutcome::Deny;
    }
    let taint_blocks = input.tainted && !matches!(input.class, EffectClass::Read);
    if policy.auto_approves(input.class) && !taint_blocks {
        GateOutcome::Allow
    } else {
        GateOutcome::Ask
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(class: EffectClass, tainted: bool) -> GateInput {
        GateInput {
            grant: true,
            class,
            budget_remaining: true,
            tainted,
        }
    }

    #[test]
    fn phase0_asks_for_anything_past_a_read() {
        let policy = Policy::phase0();
        assert_eq!(
            decide(input(EffectClass::Read, false), policy),
            GateOutcome::Allow
        );
        assert_eq!(
            decide(input(EffectClass::WriteLocal, false), policy),
            GateOutcome::Ask
        );
        assert_eq!(
            decide(input(EffectClass::External, true), policy),
            GateOutcome::Deny
        );
        assert_eq!(
            decide(input(EffectClass::Destructive, false), policy),
            GateOutcome::Ask
        );
    }

    #[test]
    fn missing_grant_or_budget_denies() {
        let policy = Policy::phase0();
        let mut denied = input(EffectClass::Read, false);
        denied.grant = false;
        assert_eq!(decide(denied, policy), GateOutcome::Deny);
        denied.grant = true;
        denied.budget_remaining = false;
        assert_eq!(decide(denied, policy), GateOutcome::Deny);
    }

    #[test]
    fn taint_blocks_auto_approval_of_external_effects() {
        let mut policy = Policy::phase0();
        policy.auto_approve_external = true;
        policy.auto_approve_destructive = true;
        // External is off in Phase 1 whatever the policy says.
        assert_eq!(
            decide(input(EffectClass::External, false), policy),
            GateOutcome::Deny
        );
        assert_eq!(
            decide(input(EffectClass::Destructive, false), policy),
            GateOutcome::Allow
        );
        assert_eq!(
            decide(input(EffectClass::Destructive, true), policy),
            GateOutcome::Ask
        );
    }

    #[test]
    fn phase1_denies_destructive_and_taint_blocks_above_read() {
        let policy = Policy::phase1();
        assert_eq!(
            decide(input(EffectClass::Destructive, false), policy),
            GateOutcome::Deny
        );
        assert_eq!(
            decide(input(EffectClass::External, false), policy),
            GateOutcome::Deny
        );
        assert_eq!(
            decide(input(EffectClass::Read, true), policy),
            GateOutcome::Allow
        );
        let mut open = Policy::phase1();
        open.auto_approve_write_local = true;
        assert_eq!(
            decide(input(EffectClass::WriteLocal, true), open),
            GateOutcome::Ask
        );
        assert_eq!(
            decide(input(EffectClass::WriteLocal, false), open),
            GateOutcome::Allow
        );
    }

    #[test]
    fn external_is_denied_in_phase1_for_every_policy() {
        let mut policies = vec![Policy::phase1(), Policy::phase0()];
        let mut open = Policy::phase1();
        open.deny_external = false;
        open.auto_approve_external = true;
        policies.push(open);
        for policy in policies {
            for tainted in [false, true] {
                assert_eq!(
                    decide(input(EffectClass::External, tainted), policy),
                    GateOutcome::Deny
                );
            }
        }
    }
}
