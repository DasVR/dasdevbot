//! Phase 1 roles. Leader, worker, and device are the topology.
//! Executor is the only role that may write the admission ledger or hold a
//! GitHub credential. Phase 1 runs one executor instance.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Leader,
    Worker,
    Device,
    Executor,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Leader => "leader",
            Role::Worker => "worker",
            Role::Device => "device",
            Role::Executor => "executor",
        }
    }
}

/// `server` is the phase 0 name for a leader candidate. It is not an executor.
/// `display` is a device-class node and is not an approval surface.
pub fn parse_role(value: &str) -> Option<Role> {
    match value {
        "leader" | "server" => Some(Role::Leader),
        "worker" => Some(Role::Worker),
        "device" | "display" => Some(Role::Device),
        "executor" => Some(Role::Executor),
        _ => None,
    }
}

pub fn may_seek_leadership(role: Role) -> bool {
    match role {
        Role::Leader | Role::Executor => true,
        Role::Worker | Role::Device => false,
    }
}

pub fn may_write_admission_ledger(role: Role) -> bool {
    match role {
        Role::Executor => true,
        Role::Leader | Role::Worker | Role::Device => false,
    }
}

/// GitHub App material stays on the executor. No other role may hold it.
pub fn may_hold_github_credential(role: Role) -> bool {
    match role {
        Role::Executor => true,
        Role::Leader | Role::Worker | Role::Device => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretRoleError {
    /// The name is not on this role's allowlist.
    NotAllowed,
    /// The name is a GitHub credential and this role is not the executor.
    ExecutorOnly,
}

const EXECUTOR_NAMES: &[&str] = &["ollama", "claude", "github", "github-app", "approval-key"];
const SHARED_NAMES: &[&str] = &["ollama", "claude", "approval-key"];

/// Allowlist bound to the daemon's role. Prefix matching is not a match:
/// `gh-app` and `secret:github/...` are refused for every role.
pub fn authorize_secret_name(role: Role, name: &str) -> Result<(), SecretRoleError> {
    let name = name.trim().to_ascii_lowercase();
    match role {
        Role::Executor => {
            if EXECUTOR_NAMES.contains(&name.as_str()) {
                Ok(())
            } else {
                Err(SecretRoleError::NotAllowed)
            }
        }
        Role::Leader | Role::Worker | Role::Device => {
            if SHARED_NAMES.contains(&name.as_str()) {
                Ok(())
            } else if name == "github" || name == "github-app" {
                Err(SecretRoleError::ExecutorOnly)
            } else {
                Err(SecretRoleError::NotAllowed)
            }
        }
    }
}

/// Device daemons do not claim server jobs. Every other role can claim.
pub fn claims_jobs(role: &str) -> bool {
    role != "device"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimMode {
    None,
    AssignedOnly,
    IncludeUnassigned,
}

pub fn claim_mode(role: &str) -> ClaimMode {
    match role {
        "device" => ClaimMode::None,
        "worker" => ClaimMode::AssignedOnly,
        _ => ClaimMode::IncludeUnassigned,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_names_are_an_allowlist_bound_to_the_role() {
        assert_eq!(authorize_secret_name(Role::Executor, "github"), Ok(()));
        assert_eq!(authorize_secret_name(Role::Executor, "github-app"), Ok(()));
        assert_eq!(authorize_secret_name(Role::Executor, "ollama"), Ok(()));
        assert_eq!(
            authorize_secret_name(Role::Executor, "approval-key"),
            Ok(())
        );
        for role in [Role::Leader, Role::Worker, Role::Device] {
            assert_eq!(
                authorize_secret_name(role, "github"),
                Err(SecretRoleError::ExecutorOnly)
            );
            assert_eq!(
                authorize_secret_name(role, "github-app"),
                Err(SecretRoleError::ExecutorOnly)
            );
            assert_eq!(authorize_secret_name(role, "ollama"), Ok(()));
            assert_eq!(authorize_secret_name(role, "claude"), Ok(()));
            assert!(!may_hold_github_credential(role));
            assert!(!may_write_admission_ledger(role));
        }
        assert!(may_hold_github_credential(Role::Executor));
        assert!(may_write_admission_ledger(Role::Executor));
        for name in ["gh-app", "secret:github/reviewer-NIL", "ollama-cloud", "anthropic"] {
            assert_eq!(
                authorize_secret_name(Role::Executor, name),
                Err(SecretRoleError::NotAllowed),
                "{name}"
            );
            assert_eq!(
                authorize_secret_name(Role::Device, name),
                Err(SecretRoleError::NotAllowed),
                "{name}"
            );
        }
    }

    #[test]
    fn server_is_a_leader_candidate_and_not_an_executor() {
        assert_eq!(parse_role("server"), Some(Role::Leader));
        assert_eq!(parse_role("executor"), Some(Role::Executor));
        assert!(may_seek_leadership(Role::Leader));
        assert!(may_seek_leadership(Role::Executor));
        assert!(!may_seek_leadership(Role::Worker));
        assert_eq!(claim_mode("worker"), ClaimMode::AssignedOnly);
        assert_eq!(claim_mode("device"), ClaimMode::None);
        assert!(!claims_jobs("device"));
        assert!(claims_jobs("server"));
    }
}
