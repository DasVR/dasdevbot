//! Decisions move only through the card-window path.
//! The daemon verifies the user, mints a single-use nonce, signs with the
//! approval key, and checks that signature before any commit. The seed never
//! leaves the secret store.

use dasdevbot_core::{
    authorize_decision, authorize_secret_name, authorize_secret_window, parse_role, EffectClass,
    SecretRoleError, Surface,
};

use crate::audit_log;
use crate::secrets::{last4, Secret, SecretError, SecretHandle};
use crate::signature::{
    self, action_hash, decision_message, payload_hash, sign_with_approval_key, undo_message,
    DECISION_PURPOSE, UNDO_PURPOSE,
};
use crate::store::{DecisionProof, DecisionRecord, Store};
use crate::verify_user::UserVerifier;
use crate::{wall_ms, Error, Result};

pub struct SignRequest<'a> {
    pub window: &'a str,
    pub voice: bool,
    pub approval_id: &'a str,
    pub decision: &'a str,
    pub reason: Option<&'a str>,
    pub now_ms: u64,
    pub fencing: u64,
    pub secrets: &'a dyn SecretHandle,
    pub verifier: &'a dyn UserVerifier,
    pub audit_seed: &'a [u8; 32],
}

pub struct UndoRequest<'a> {
    pub window: &'a str,
    pub voice: bool,
    pub approval_id: &'a str,
    pub now_ms: u64,
    pub fencing: u64,
    pub secrets: &'a dyn SecretHandle,
    pub verifier: &'a dyn UserVerifier,
    pub audit_seed: &'a [u8; 32],
}

pub fn sign_decision(store: &mut Store, request: SignRequest<'_>) -> Result<DecisionRecord> {
    let (class_name, action, draft) = store.approval_material(request.approval_id)?;
    let class = EffectClass::parse(&class_name)
        .ok_or_else(|| Error::BadRequest("unknown effect class".into()))?;
    let surface = if request.voice {
        Surface::Voice
    } else {
        Surface::TauriIpc
    };
    authorize_decision(surface, request.window, request.voice, class)
        .map_err(|deny| Error::Forbidden(format!("decision refused: {deny:?}")))?;
    verify_user(request.verifier)?;
    let reason = request.reason.unwrap_or("");
    let action_hash = action_hash(&class_name, &action, &draft);
    let payload_hash = payload_hash(
        DECISION_PURPOSE,
        request.approval_id,
        request.decision,
        reason,
        request.window,
        &action_hash,
    );
    let nonce = store.issue_decision_nonce(
        request.approval_id,
        &payload_hash,
        DECISION_PURPOSE,
        request.now_ms,
    )?;
    let message = decision_message(
        request.approval_id,
        request.decision,
        reason,
        request.window,
        &nonce,
        request.fencing,
        &action_hash,
    );
    let signature = sign_with_approval_key(request.secrets, &message)
        .ok_or_else(|| Error::Forbidden("approval key is not configured".into()))?;
    if !signature::verify_decision_signature(request.secrets, &message, &signature) {
        return Err(Error::Forbidden(
            "decision signature is missing or invalid".into(),
        ));
    }
    let record = store.commit_signed_decision(
        request.approval_id,
        request.decision,
        request.reason,
        request.now_ms,
        &DecisionProof {
            signature,
            nonce,
            fencing: request.fencing,
            action_hash,
            window: request.window.to_string(),
        },
        request.secrets,
    )?;
    let audit_payload = serde_json::json!({
        "approval_id": request.approval_id,
        "decision": request.decision,
        "window": request.window,
    })
    .to_string();
    audit_log::append(
        store,
        "approval.decided",
        &audit_payload,
        request.now_ms,
        request.audit_seed,
    )?;
    Ok(record)
}

pub fn undo_decision(store: &mut Store, request: UndoRequest<'_>) -> Result<DecisionRecord> {
    let (class_name, action, draft) = store.approval_material(request.approval_id)?;
    let class = EffectClass::parse(&class_name).unwrap_or(EffectClass::External);
    authorize_decision(
        if request.voice {
            Surface::Voice
        } else {
            Surface::TauriIpc
        },
        request.window,
        request.voice,
        class,
    )
    .map_err(|deny| Error::Forbidden(format!("undo refused: {deny:?}")))?;
    verify_user(request.verifier)?;
    let status = store.approval_status(request.approval_id)?;
    let action_hash = action_hash(&class_name, &action, &draft);
    let payload_hash = payload_hash(
        UNDO_PURPOSE,
        request.approval_id,
        "undo",
        &status,
        request.window,
        &action_hash,
    );
    let nonce = store.issue_decision_nonce(
        request.approval_id,
        &payload_hash,
        UNDO_PURPOSE,
        request.now_ms,
    )?;
    let message = undo_message(
        request.approval_id,
        &status,
        request.window,
        &nonce,
        request.fencing,
        &action_hash,
    );
    let signature = sign_with_approval_key(request.secrets, &message)
        .ok_or_else(|| Error::Forbidden("approval key is not configured".into()))?;
    if !signature::verify_decision_signature(request.secrets, &message, &signature) {
        return Err(Error::Forbidden(
            "undo signature is missing or invalid".into(),
        ));
    }
    let record = store.commit_signed_undo(
        request.approval_id,
        request.now_ms,
        &DecisionProof {
            signature,
            nonce,
            fencing: request.fencing,
            action_hash,
            window: request.window.to_string(),
        },
        request.secrets,
    )?;
    let audit_payload = serde_json::json!({
        "approval_id": request.approval_id,
        "window": request.window,
    })
    .to_string();
    audit_log::append(
        store,
        "approval.undone",
        &audit_payload,
        request.now_ms,
        request.audit_seed,
    )?;
    Ok(record)
}

fn verify_user(verifier: &dyn UserVerifier) -> Result<()> {
    verifier
        .verify_user("Confirm this dasdevbot card")
        .map_err(|err| Error::Forbidden(format!("user verification failed: {err}")))
}

pub fn authorize_secret_entry(window: &str) -> Result<()> {
    authorize_secret_window(window).map_err(|_| {
        Error::Forbidden("secret entry is scoped to the settings or main window".into())
    })
}

/// `role` is the daemon's own configured role. The caller does not supply one.
pub fn store_secret(
    role: &str,
    window: &str,
    name: &str,
    secret: &Secret,
    secrets: &dyn SecretHandle,
    store: &mut Store,
    audit_seed: &[u8; 32],
) -> Result<String> {
    authorize_secret_entry(window)?;
    let Some(role) = parse_role(role) else {
        return Err(Error::Forbidden("daemon role is not configured".into()));
    };
    authorize_secret_name(role, name).map_err(|err| match err {
        SecretRoleError::NotAllowed => {
            Error::Forbidden("secret name is not on this role's allowlist".into())
        }
        SecretRoleError::ExecutorOnly => {
            Error::Forbidden("only the executor may store a GitHub credential".into())
        }
    })?;
    secrets.set(name, secret).map_err(secret_error)?;
    let tail = last4(secret);
    let audit_payload = serde_json::json!({
        "name": name,
        "last4": tail,
    })
    .to_string();
    audit_log::append(store, "secret.set", &audit_payload, wall_ms(), audit_seed)?;
    Ok(tail)
}

fn secret_error(err: SecretError) -> Error {
    match err {
        SecretError::Refused(text) => Error::Forbidden(text),
        SecretError::Unavailable(text) => Error::BadRequest(text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::{MemorySecrets, APPROVAL_KEY_NAME};
    use crate::store::NewApproval;
    use crate::verify_user::TestVerifier;
    use dasdevbot_core::{authorize_decision, DecisionDeny, Surface, CARD_WINDOW, MAIN_WINDOW};

    const TEST_SEED: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
    const AUDIT_SEED: [u8; 32] = [4u8; 32];

    fn secrets() -> MemorySecrets {
        let secrets = MemorySecrets::new();
        secrets.insert(APPROVAL_KEY_NAME, TEST_SEED);
        secrets
    }

    fn allow() -> TestVerifier {
        TestVerifier { allow: true }
    }

    fn pending(store: &mut Store, class: &str) -> String {
        let now = 1_000;
        let job_id = store
            .enqueue_job(
                "reviewer",
                &format!("job-{class}-{}", uuid::Uuid::new_v4()),
                "{}",
                now,
            )
            .unwrap()
            .unwrap();
        store.claim_at("owner", now, 60_000).unwrap().unwrap();
        store
            .record_approval_and_wait(
                now,
                "owner",
                NewApproval {
                    job_id,
                    agent_id: "reviewer".into(),
                    thread_id: "thread".into(),
                    effect_class: class.into(),
                    action: "post_pr_comment".into(),
                    purpose: "purpose".into(),
                    draft: "draft".into(),
                    evidence: "evidence".into(),
                    evidence_repo: "DasVR/NIL".into(),
                    evidence_ref: "phase0".into(),
                    evidence_event_id: "ev".into(),
                    evidence_kind: "repo.push".into(),
                    provider: "fake".into(),
                    model: "fake".into(),
                    usage_kind: "estimated".into(),
                    input_tokens: 1,
                    output_tokens: 1,
                    micro_usd: 0,
                    ledger_note: "fake".into(),
                    project: "DasVR/NIL".into(),
                },
                "{}",
                "{}",
            )
            .unwrap()
    }

    fn sign<'a>(
        store: &mut Store,
        secrets: &'a MemorySecrets,
        verifier: &'a TestVerifier,
        id: &'a str,
        window: &'a str,
        voice: bool,
        decision: &'a str,
    ) -> Result<DecisionRecord> {
        sign_decision(
            store,
            SignRequest {
                window,
                voice,
                approval_id: id,
                decision,
                reason: None,
                now_ms: 2_000,
                fencing: 7,
                secrets,
                verifier,
                audit_seed: &AUDIT_SEED,
            },
        )
    }

    #[test]
    fn the_card_window_signs_a_real_decision_and_other_paths_cannot() {
        let mut store = Store::open_memory().unwrap();
        let keys = secrets();
        let verifier = allow();
        let id = pending(&mut store, "external");
        let err = sign(&mut store, &keys, &verifier, &id, MAIN_WINDOW, false, "approve").unwrap_err();
        assert!(matches!(err, Error::Forbidden(_)));
        let err = sign(&mut store, &keys, &verifier, &id, CARD_WINDOW, true, "approve").unwrap_err();
        assert!(matches!(err, Error::Forbidden(_)));
        assert_eq!(
            authorize_decision(Surface::Http, CARD_WINDOW, false, EffectClass::External),
            Err(DecisionDeny::Http)
        );
        let missing = MemorySecrets::new();
        let err = sign(
            &mut store,
            &missing,
            &verifier,
            &id,
            CARD_WINDOW,
            false,
            "approve",
        )
        .unwrap_err();
        assert!(matches!(err, Error::Forbidden(_)));
        let denied = TestVerifier { allow: false };
        let err = sign(&mut store, &keys, &denied, &id, CARD_WINDOW, false, "approve").unwrap_err();
        assert!(err.to_string().contains("user verification"));
        assert_eq!(store.approval_status(&id).unwrap(), "pending");

        let signed = sign(&mut store, &keys, &verifier, &id, CARD_WINDOW, false, "approve").unwrap();
        assert_eq!(signed.status, "approved");
        let payload: String = store
            .connection()
            .query_row(
                "SELECT payload FROM events WHERE id = ?1",
                [signed.event_id.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        let body: serde_json::Value = serde_json::from_str(&payload).unwrap();
        let signature = body["signature"].as_str().unwrap();
        let nonce = body["nonce"].as_str().unwrap();
        let action_hash = body["action_hash"].as_str().unwrap();
        assert_eq!(body["fencing"], 7);
        assert_eq!(body["window"], CARD_WINDOW);
        assert_eq!(action_hash, super::action_hash("external", "post_pr_comment", "draft"));
        let message = decision_message(&id, "approve", "", CARD_WINDOW, nonce, 7, action_hash);
        assert!(signature::verify_decision_signature(&keys, &message, signature));
        let tampered = decision_message(&id, "deny", "", CARD_WINDOW, nonce, 7, action_hash);
        assert!(!signature::verify_decision_signature(&keys, &tampered, signature));
        let replay = store
            .commit_signed_decision(
                &id,
                "approve",
                None,
                2_000,
                &DecisionProof {
                    signature: signature.to_string(),
                    nonce: nonce.to_string(),
                    fencing: 7,
                    action_hash: action_hash.to_string(),
                    window: CARD_WINDOW.to_string(),
                },
                &keys,
            )
            .unwrap();
        assert_eq!(replay.status, "approved");
        let decided_rows: i64 = store
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM events WHERE kind = 'approval.decided'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(decided_rows, 1);

        let forged = pending(&mut store, "external");
        let err = store
            .commit_signed_decision(
                &forged,
                "approve",
                None,
                2_000,
                &DecisionProof {
                    signature: String::new(),
                    nonce: "missing".into(),
                    fencing: 7,
                    action_hash: action_hash.to_string(),
                    window: CARD_WINDOW.to_string(),
                },
                &keys,
            )
            .unwrap_err();
        assert!(matches!(err, Error::Forbidden(_)));
        assert_eq!(store.approval_status(&forged).unwrap(), "pending");

        let destructive = pending(&mut store, "destructive");
        let err = sign(
            &mut store,
            &keys,
            &verifier,
            &destructive,
            CARD_WINDOW,
            false,
            "approve",
        )
        .unwrap_err();
        assert!(matches!(err, Error::Forbidden(_)));
        assert_eq!(store.approval_effect_class(&destructive).unwrap(), "destructive");
        let kinds: Vec<String> = store
            .connection()
            .prepare("SELECT kind FROM audit_log")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert!(kinds.iter().any(|kind| kind == "approval.decided"));
        assert!(audit_log::verify(&store, &AUDIT_SEED).unwrap());
    }

    #[test]
    fn undo_is_signed_and_a_missing_signature_does_not_revert() {
        let mut store = Store::open_memory().unwrap();
        let keys = secrets();
        let verifier = allow();
        let id = pending(&mut store, "external");
        sign(&mut store, &keys, &verifier, &id, CARD_WINDOW, false, "approve").unwrap();
        let err = undo_decision(
            &mut store,
            UndoRequest {
                window: CARD_WINDOW,
                voice: false,
                approval_id: &id,
                now_ms: 2_500,
                fencing: 7,
                secrets: &keys,
                verifier: &TestVerifier { allow: false },
                audit_seed: &AUDIT_SEED,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("user verification"));
        assert_eq!(store.approval_status(&id).unwrap(), "approved");
        let undone = undo_decision(
            &mut store,
            UndoRequest {
                window: CARD_WINDOW,
                voice: false,
                approval_id: &id,
                now_ms: 2_500,
                fencing: 7,
                secrets: &keys,
                verifier: &verifier,
                audit_seed: &AUDIT_SEED,
            },
        )
        .unwrap();
        assert_eq!(undone.status, "pending");
    }

    #[test]
    fn secret_storage_uses_the_daemon_role_allowlist() {
        let mut store = Store::open_memory().unwrap();
        let keys = MemorySecrets::new();
        let github = Secret::new("gh-token-value");
        let err = store_secret(
            "device",
            "settings",
            "github",
            &github,
            &keys,
            &mut store,
            &AUDIT_SEED,
        )
        .unwrap_err();
        assert!(matches!(err, Error::Forbidden(_)));
        let err = store_secret(
            "device",
            "settings",
            "gh-app",
            &github,
            &keys,
            &mut store,
            &AUDIT_SEED,
        )
        .unwrap_err();
        assert!(matches!(err, Error::Forbidden(_)));
        let err = store_secret("", "settings", "ollama", &github, &keys, &mut store, &AUDIT_SEED)
            .unwrap_err();
        assert!(err.to_string().contains("role"));
        let err = store_secret(
            "executor",
            "card",
            "github",
            &github,
            &keys,
            &mut store,
            &AUDIT_SEED,
        )
        .unwrap_err();
        assert!(matches!(err, Error::Forbidden(_)));
        let tail = store_secret(
            "executor",
            "settings",
            "github",
            &github,
            &keys,
            &mut store,
            &AUDIT_SEED,
        )
        .unwrap();
        assert_eq!(tail, "alue");
        let stored = keys.get("github").unwrap().unwrap();
        assert_eq!(stored.expose(), "gh-token-value");
        let payload: String = store
            .connection()
            .query_row(
                "SELECT payload FROM audit_log WHERE kind = 'secret.set'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(payload.contains("alue"));
        assert!(!payload.contains("gh-token-value"));
        assert!(audit_log::verify(&store, &AUDIT_SEED).unwrap());
    }
}
