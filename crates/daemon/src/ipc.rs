//! Decisions move only through the card-window path.
//! Internal tiers are signed with the approval seed. External tiers require a
//! Windows Hello signature over a daemon nonce; the daemon verifies the stored
//! public key and does not sign that tier with the seed.

use dasdevbot_core::{
    authorize_decision, authorize_secret_name, authorize_secret_window, parse_role, EffectClass,
    SecretRoleError, Surface,
};

use crate::audit_log;
use crate::hello_key::{self, consent_prompt};
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
    /// Set only for an external card. The daemon does not mint this signature.
    pub client_signature: Option<&'a str>,
    /// Nonce from [`prepare_signature`], required with `client_signature`.
    pub client_nonce: Option<&'a str>,
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
    pub client_signature: Option<&'a str>,
    pub client_nonce: Option<&'a str>,
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
    if class == EffectClass::External {
        return accept_external_decision(store, request, &class_name, &action, &draft);
    }
    let target = store.approval_target(request.approval_id)?;
    verify_user(
        request.verifier,
        &consent_prompt(request.decision, &action, &target),
    )?;
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
    if class == EffectClass::External {
        return accept_external_undo(store, request, &class_name, &action, &draft);
    }
    let target = store.approval_target(request.approval_id)?;
    verify_user(
        request.verifier,
        &consent_prompt("undo", &action, &target),
    )?;
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

fn verify_user(verifier: &dyn UserVerifier, prompt: &str) -> Result<()> {
    verifier
        .verify_user(prompt)
        .map_err(|err| Error::Forbidden(format!("user verification failed: {err}")))
}

fn accept_external_decision(
    store: &mut Store,
    request: SignRequest<'_>,
    class_name: &str,
    action: &str,
    draft: &str,
) -> Result<DecisionRecord> {
    if !hello_key::signing_path_is_present() {
        return Err(Error::Forbidden(
            "external tier is denied without Windows Hello".into(),
        ));
    }
    let signature = request.client_signature.filter(|text| !text.is_empty()).ok_or_else(|| {
        Error::Forbidden("external tier requires a Windows Hello signature".into())
    })?;
    let nonce = request.client_nonce.filter(|text| !text.is_empty()).ok_or_else(|| {
        Error::Forbidden("external tier requires a Windows Hello signature".into())
    })?;
    let target = store.approval_target(request.approval_id)?;
    verify_user(
        request.verifier,
        &consent_prompt(request.decision, action, &target),
    )?;
    let action_hash = action_hash(class_name, action, draft);
    let record = store.commit_signed_decision(
        request.approval_id,
        request.decision,
        request.reason,
        request.now_ms,
        &DecisionProof {
            signature: signature.to_string(),
            nonce: nonce.to_string(),
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

fn accept_external_undo(
    store: &mut Store,
    request: UndoRequest<'_>,
    class_name: &str,
    action: &str,
    draft: &str,
) -> Result<DecisionRecord> {
    if !hello_key::signing_path_is_present() {
        return Err(Error::Forbidden(
            "external tier is denied without Windows Hello".into(),
        ));
    }
    let signature = request.client_signature.filter(|text| !text.is_empty()).ok_or_else(|| {
        Error::Forbidden("external tier requires a Windows Hello signature".into())
    })?;
    let nonce = request.client_nonce.filter(|text| !text.is_empty()).ok_or_else(|| {
        Error::Forbidden("external tier requires a Windows Hello signature".into())
    })?;
    let target = store.approval_target(request.approval_id)?;
    verify_user(request.verifier, &consent_prompt("undo", action, &target))?;
    let action_hash = action_hash(class_name, action, draft);
    let record = store.commit_signed_undo(
        request.approval_id,
        request.now_ms,
        &DecisionProof {
            signature: signature.to_string(),
            nonce: nonce.to_string(),
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

pub struct PrepareRequest<'a> {
    pub approval_id: &'a str,
    pub decision: &'a str,
    pub reason: &'a str,
    pub window: &'a str,
    pub fencing: u64,
    pub now_ms: u64,
    pub purpose: &'a str,
}

/// Issue the nonce an external signature must cover. Internal cards do not need it.
pub fn prepare_signature(store: &mut Store, request: PrepareRequest<'_>) -> Result<serde_json::Value> {
    let PrepareRequest {
        approval_id,
        decision,
        reason,
        window,
        fencing,
        now_ms,
        purpose,
    } = request;
    let (class_name, action, draft) = store.approval_material(approval_id)?;
    let class = EffectClass::parse(&class_name)
        .ok_or_else(|| Error::BadRequest("unknown effect class".into()))?;
    if class != EffectClass::External {
        return Ok(serde_json::json!({
            "signature_required": false,
            "tier": "internal",
        }));
    }
    if !hello_key::signing_path_is_present() {
        return Err(Error::Forbidden(
            "external tier is denied without Windows Hello".into(),
        ));
    }
    let target = store.approval_target(approval_id)?;
    let action_hash = action_hash(&class_name, &action, &draft);
    let prompt_decision = if purpose == UNDO_PURPOSE { "undo" } else { decision };
    let prompt = consent_prompt(prompt_decision, &action, &target);
    let (message, nonce) = if purpose == UNDO_PURPOSE {
        let status = store.approval_status(approval_id)?;
        let payload_hash = payload_hash(
            UNDO_PURPOSE,
            approval_id,
            "undo",
            &status,
            window,
            &action_hash,
        );
        let nonce = store.issue_decision_nonce(approval_id, &payload_hash, UNDO_PURPOSE, now_ms)?;
        (
            undo_message(approval_id, &status, window, &nonce, fencing, &action_hash),
            nonce,
        )
    } else {
        let payload_hash = payload_hash(
            DECISION_PURPOSE,
            approval_id,
            decision,
            reason,
            window,
            &action_hash,
        );
        let nonce =
            store.issue_decision_nonce(approval_id, &payload_hash, DECISION_PURPOSE, now_ms)?;
        (
            decision_message(
                approval_id,
                decision,
                reason,
                window,
                &nonce,
                fencing,
                &action_hash,
            ),
            nonce,
        )
    };
    Ok(serde_json::json!({
        "signature_required": true,
        "tier": "external",
        "nonce": nonce,
        "message": message,
        "prompt": prompt,
        "hello_enrolled": store.hello_public_key()?.is_some(),
    }))
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
    let tail = last4(secret);
    let audit_payload = serde_json::json!({
        "name": name,
        "last4": tail,
    })
    .to_string();
    audit_log::append(store, "secret.set", &audit_payload, wall_ms(), audit_seed)?;
    secrets.set(name, secret).map_err(secret_error)?;
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
    use crate::secrets::{MemorySecrets, Secret, SecretError, SecretHandle, APPROVAL_KEY_NAME};
    use crate::store::NewApproval;
    use crate::verify_user::{TestVerifier, UserVerifier, UserVerifyError};
    use dasdevbot_core::{authorize_decision, DecisionDeny, EffectClass, Surface, CARD_WINDOW, MAIN_WINDOW};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

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
                client_signature: None,
                client_nonce: None,
            },
        )
    }

    #[test]
    fn the_card_window_signs_a_real_decision_and_other_paths_cannot() {
        let mut store = Store::open_memory().unwrap();
        let keys = secrets();
        let verifier = allow();
        let id = pending(&mut store, "write_local");
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
        assert_eq!(action_hash, super::action_hash("write_local", "post_pr_comment", "draft"));
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

        let forged = pending(&mut store, "write_local");
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
        let id = pending(&mut store, "write_local");
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
                client_signature: None,
                client_nonce: None,
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
                client_signature: None,
                client_nonce: None,
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

    #[test]
    fn external_seed_cannot_approve() {
        let mut store = Store::open_memory().unwrap();
        let id = pending(&mut store, "external");
        let err = sign(
            &mut store,
            &secrets(),
            &allow(),
            &id,
            CARD_WINDOW,
            false,
            "approve",
        )
        .unwrap_err();
        assert!(err.to_string().contains("Windows Hello"), "{err}");
        assert_eq!(store.approval_status(&id).unwrap(), "pending");
    }

    struct RecordingVerifier {
        prompt: Mutex<String>,
    }

    impl UserVerifier for RecordingVerifier {
        fn verify_user(&self, prompt: &str) -> std::result::Result<(), UserVerifyError> {
            *self.prompt.lock().expect("prompt") = prompt.to_string();
            Ok(())
        }
    }

    #[test]
    fn the_consent_prompt_names_the_action_and_the_target() {
        let mut store = Store::open_memory().unwrap();
        let id = pending(&mut store, "write_local");
        let keys = secrets();
        let verifier = RecordingVerifier {
            prompt: Mutex::new(String::new()),
        };
        sign_decision(
            &mut store,
            SignRequest {
                window: CARD_WINDOW,
                voice: false,
                approval_id: &id,
                decision: "approve",
                reason: None,
                now_ms: 2_000,
                fencing: 7,
                secrets: &keys,
                verifier: &verifier,
                audit_seed: &AUDIT_SEED,
                client_signature: None,
                client_nonce: None,
            },
        )
        .unwrap();
        let prompt = verifier.prompt.lock().expect("prompt").clone();
        assert!(prompt.contains("approve"), "{prompt}");
        assert!(prompt.contains("post_pr_comment"), "{prompt}");
        assert!(prompt.contains("DasVR/NIL"), "{prompt}");
        assert!(!prompt.contains("Confirm this dasdevbot card"), "{prompt}");
    }

    struct CountingSecrets {
        inner: MemorySecrets,
        sets: AtomicUsize,
    }

    impl SecretHandle for CountingSecrets {
        fn get(&self, name: &str) -> std::result::Result<Option<Secret>, SecretError> {
            self.inner.get(name)
        }

        fn set(&self, name: &str, secret: &Secret) -> std::result::Result<(), SecretError> {
            self.sets.fetch_add(1, Ordering::SeqCst);
            self.inner.set(name, secret)
        }
    }

    #[test]
    fn a_failed_audit_does_not_write_the_keyring() {
        let mut store = Store::open_memory().unwrap();
        store
            .connection()
            .execute_batch("DROP TABLE audit_log;")
            .unwrap();
        let keys = CountingSecrets {
            inner: MemorySecrets::new(),
            sets: AtomicUsize::new(0),
        };
        let secret = Secret::new("gh-token-value");
        let err = store_secret(
            "executor",
            "settings",
            "ollama",
            &secret,
            &keys,
            &mut store,
            &AUDIT_SEED,
        )
        .unwrap_err();
        assert!(err.to_string().contains("audit_log") || err.to_string().contains("sqlite"), "{err}");
        assert_eq!(keys.sets.load(Ordering::SeqCst), 0);
        assert!(keys.get("ollama").unwrap().is_none());
    }
}
