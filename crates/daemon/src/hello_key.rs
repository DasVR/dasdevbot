//! External-tier approvals. The private key is a non-exportable Windows Hello
//! key created with `KeyCredentialManager`. Those keys are RSA-2048, and
//! `RequestSignAsync` signs SHA-256 with RSASSA-PKCS1-v1_5. The daemon stores
//! only the X.509 SubjectPublicKeyInfo and verifies with CNG using
//! `BCRYPT_PAD_PKCS1` + SHA-256. Other targets have no such key, so the
//! external tier stays denied. Internal tiers keep the Phase 1 Ed25519 path.
//!
//! Phase 1 denies the external tier in the policy itself
//! (`dasdevbot_core::EXTERNAL_TIER_ENABLED`). This code is not reachable from
//! a decision until the hardware test in docs/hello-hardware-test.md passes.

use crate::store::Store;
use crate::verify_user::user_verification_is_real;
use crate::Result;

#[cfg_attr(not(windows), allow(dead_code))]
pub const HELLO_KEY_NAME: &str = "dasdevbot-approval";
/// `CryptographicPublicKeyBlobType::X509SubjectPublicKeyInfo`: DER SPKI of the
/// RSA-2048 Hello key.
#[cfg_attr(not(windows), allow(dead_code))]
pub const X509_SPKI_BLOB: i64 = 0;

/// True only when this binary contains the Hello key path.
pub fn signing_path_is_present() -> bool {
    user_verification_is_real()
}

/// The consent dialog names the decision, the action, and the target.
pub fn consent_prompt(decision: &str, action: &str, target: &str) -> String {
    let decision = if decision.is_empty() {
        "confirm"
    } else {
        decision
    };
    let action = if action.is_empty() { "action" } else { action };
    let target = if target.is_empty() { "target" } else { target };
    format!("{decision} {action} on {target}")
}

/// blake3 of the blob type and the lowercased hex key. The pin in the audit
/// tip sidecar holds this value.
pub fn hello_fingerprint(blob_type: i64, public_key_hex: &str) -> String {
    let key = public_key_hex.to_ascii_lowercase();
    blake3::hash(format!("{blob_type}\n{key}").as_bytes())
        .to_hex()
        .to_string()
}

/// The stored key, only when its fingerprint equals the pin.
pub(crate) fn pinned_public_key(store: &Store) -> Result<Option<(i64, String)>> {
    let Some(pin) = crate::audit_log::read_hello_pin(store)? else {
        return Ok(None);
    };
    let Some((blob_type, public_key)) = store.hello_public_key()? else {
        return Ok(None);
    };
    if hello_fingerprint(blob_type, &public_key) != pin {
        return Ok(None);
    }
    Ok(Some((blob_type, public_key)))
}

pub fn verify_stored(store: &Store, message: &str, signature_hex: &str) -> Result<bool> {
    if !signing_path_is_present() {
        return Ok(false);
    }
    let Some((blob_type, public_key)) = pinned_public_key(store)? else {
        return Ok(false);
    };
    let Some(public_key) = crate::signature::decode_hex_bytes(&public_key) else {
        return Ok(false);
    };
    let Some(signature) = crate::signature::decode_hex_bytes(signature_hex) else {
        return Ok(false);
    };
    Ok(verify_cng(
        blob_type,
        &public_key,
        message.as_bytes(),
        &signature,
    ))
}

#[cfg(not(windows))]
fn verify_cng(_blob_type: i64, _public_key: &[u8], _message: &[u8], _signature: &[u8]) -> bool {
    false
}

#[cfg(windows)]
fn verify_cng(blob_type: i64, public_key: &[u8], message: &[u8], signature: &[u8]) -> bool {
    cng::verify(blob_type, public_key, message, signature)
}

/// Open or create the Hello key and return `(blob_type, public_key_hex)`.
/// The private key never leaves the platform.
#[cfg(windows)]
pub fn hello_public_key_material() -> std::result::Result<(i64, String), String> {
    let credential = win::open_or_create()?;
    let bytes = win::public_key_bytes(&credential)?;
    Ok((X509_SPKI_BLOB, crate::signature::hex_encode(&bytes)))
}

/// Sign `message` with the non-exportable Hello key. `prompt` is the consent
/// text and must already name the action and the target.
#[cfg(windows)]
pub fn sign_approval_message(message: &str, prompt: &str) -> std::result::Result<String, String> {
    win::consent(prompt)?;
    let credential = win::open_or_create()?;
    let signature = win::sign(&credential, message)?;
    Ok(crate::signature::hex_encode(&signature))
}

#[cfg(windows)]
mod win {
    use super::HELLO_KEY_NAME;
    use windows::core::HSTRING;
    use windows::Security::Credentials::UI::{
        UserConsentVerificationResult, UserConsentVerifier, UserConsentVerifierAvailability,
    };
    use windows::Security::Credentials::{
        KeyCredential, KeyCredentialCreationOption, KeyCredentialManager, KeyCredentialStatus,
    };
    use windows::Security::Cryptography::Core::CryptographicPublicKeyBlobType;
    use windows::Security::Cryptography::{BinaryStringEncoding, CryptographicBuffer};
    use windows::Storage::Streams::DataReader;

    pub fn consent(prompt: &str) -> Result<(), String> {
        let availability = UserConsentVerifier::CheckAvailabilityAsync()
            .map_err(|err| err.to_string())?
            .get()
            .map_err(|err| err.to_string())?;
        if availability != UserConsentVerifierAvailability::Available {
            return Err("Windows Hello is not available".into());
        }
        let result = UserConsentVerifier::RequestVerificationAsync(&HSTRING::from(prompt))
            .map_err(|err| err.to_string())?
            .get()
            .map_err(|err| err.to_string())?;
        if result == UserConsentVerificationResult::Verified {
            Ok(())
        } else {
            Err("Windows Hello consent was denied".into())
        }
    }

    pub fn open_or_create() -> Result<KeyCredential, String> {
        let name = HSTRING::from(HELLO_KEY_NAME);
        let opened = KeyCredentialManager::OpenAsync(&name)
            .map_err(|err| err.to_string())?
            .get()
            .map_err(|err| err.to_string())?;
        if opened.Status().map_err(|err| err.to_string())? == KeyCredentialStatus::Success {
            return opened.Credential().map_err(|err| err.to_string());
        }
        let created = KeyCredentialManager::RequestCreateAsync(
            &name,
            KeyCredentialCreationOption::FailIfExists,
        )
        .map_err(|err| err.to_string())?
        .get()
        .map_err(|err| err.to_string())?;
        if created.Status().map_err(|err| err.to_string())? != KeyCredentialStatus::Success {
            return Err("Windows Hello key was not created".into());
        }
        created.Credential().map_err(|err| err.to_string())
    }

    pub fn public_key_bytes(credential: &KeyCredential) -> Result<Vec<u8>, String> {
        let buffer = credential
            .RetrievePublicKeyWithBlobType(CryptographicPublicKeyBlobType::X509SubjectPublicKeyInfo)
            .map_err(|err| err.to_string())?;
        buffer_bytes(&buffer)
    }

    pub fn sign(credential: &KeyCredential, message: &str) -> Result<Vec<u8>, String> {
        let data = CryptographicBuffer::ConvertStringToBinary(
            &HSTRING::from(message),
            BinaryStringEncoding::Utf8,
        )
        .map_err(|err| err.to_string())?;
        let signed = credential
            .RequestSignAsync(&data)
            .map_err(|err| err.to_string())?
            .get()
            .map_err(|err| err.to_string())?;
        if signed.Status().map_err(|err| err.to_string())? != KeyCredentialStatus::Success {
            return Err("Windows Hello refused to sign".into());
        }
        let buffer = signed.Result().map_err(|err| err.to_string())?;
        buffer_bytes(&buffer)
    }

    fn buffer_bytes(buffer: &windows::Storage::Streams::IBuffer) -> Result<Vec<u8>, String> {
        let reader = DataReader::FromBuffer(buffer).map_err(|err| err.to_string())?;
        let len = reader
            .UnconsumedBufferLength()
            .map_err(|err| err.to_string())? as usize;
        let mut bytes = vec![0u8; len];
        reader
            .ReadBytes(&mut bytes)
            .map_err(|err| err.to_string())?;
        Ok(bytes)
    }
}

#[cfg(windows)]
mod cng {
    //! RSA-2048 PKCS#1 v1.5 / SHA-256 verify of a Hello signature.
    use sha2::Digest;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        BCryptDestroyKey, BCryptVerifySignature, CryptDecodeObjectEx, CryptImportPublicKeyInfoEx2,
        BCRYPT_KEY_HANDLE, BCRYPT_PAD_PKCS1, BCRYPT_PKCS1_PADDING_INFO, BCRYPT_SHA256_ALGORITHM,
        CERT_PUBLIC_KEY_INFO, CRYPT_DECODE_ALLOC_FLAG, CRYPT_IMPORT_PUBLIC_KEY_FLAGS,
        X509_ASN_ENCODING, X509_PUBLIC_KEY_INFO,
    };

    /// OID of rsaEncryption. Anything else is refused.
    const RSA_OID: &[u8] = b"1.2.840.113549.1.1.1";

    pub fn verify(blob_type: i64, spki: &[u8], message: &[u8], signature: &[u8]) -> bool {
        if blob_type != super::X509_SPKI_BLOB || signature.len() != 256 {
            return false;
        }
        let digest = sha2::Sha256::digest(message);
        // SAFETY: every pointer below is either owned here or freed before return.
        unsafe { verify_rsa(spki, digest.as_slice(), signature) }
    }

    unsafe fn verify_rsa(spki: &[u8], digest: &[u8], signature: &[u8]) -> bool {
        let mut info: *mut CERT_PUBLIC_KEY_INFO = std::ptr::null_mut();
        let mut info_len = 0u32;
        let decoded = unsafe {
            CryptDecodeObjectEx(
                X509_ASN_ENCODING,
                X509_PUBLIC_KEY_INFO,
                spki,
                CRYPT_DECODE_ALLOC_FLAG,
                None,
                Some(&mut info as *mut *mut CERT_PUBLIC_KEY_INFO as *mut core::ffi::c_void),
                &mut info_len,
            )
        };
        if decoded.is_err() || info.is_null() {
            return false;
        }
        let oid = unsafe { (*info).Algorithm.pszObjId };
        let is_rsa = !oid.is_null() && unsafe { oid.as_bytes() } == RSA_OID;
        let mut key = BCRYPT_KEY_HANDLE(std::ptr::null_mut());
        let imported = is_rsa
            && unsafe {
                CryptImportPublicKeyInfoEx2(
                    X509_ASN_ENCODING,
                    info,
                    CRYPT_IMPORT_PUBLIC_KEY_FLAGS(0),
                    None,
                    &mut key,
                )
            }
            .is_ok();
        unsafe {
            let _ = LocalFree(Some(HLOCAL(info as *mut core::ffi::c_void)));
        }
        if !imported {
            return false;
        }
        let padding = BCRYPT_PKCS1_PADDING_INFO {
            pszAlgId: BCRYPT_SHA256_ALGORITHM,
        };
        let status = unsafe {
            BCryptVerifySignature(
                key,
                Some(&padding as *const BCRYPT_PKCS1_PADDING_INFO as *const core::ffi::c_void),
                digest,
                signature,
                BCRYPT_PAD_PKCS1,
            )
        };
        unsafe {
            let _ = BCryptDestroyKey(key);
        }
        status.0 == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit_log;

    const SEED: [u8; 32] = [7u8; 32];
    const KEY_A: &str = "30820122300d06092a864886f70d01010105000382010f003082010a0282010100aa";
    const KEY_B: &str = "30820122300d06092a864886f70d01010105000382010f003082010a0282010100bb";

    fn payloads(store: &Store, kind: &str) -> Vec<String> {
        let mut stmt = store
            .connection()
            .prepare("SELECT payload FROM audit_log WHERE kind = ?1 ORDER BY seq")
            .unwrap();
        let rows = stmt
            .query_map([kind], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|row| row.unwrap())
            .collect();
        rows
    }

    fn temp_data(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("dasdevbot-{label}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("db.sqlite")
    }

    fn plant_row(store: &Store, key: &str) {
        store
            .connection()
            .execute(
                "INSERT INTO hello_public_key (id, blob_type, public_key, created_at) VALUES (1, ?1, ?2, 30)",
                rusqlite::params![X509_SPKI_BLOB, key],
            )
            .unwrap();
    }

    #[test]
    fn the_fingerprint_ignores_hex_case_and_covers_the_blob_type() {
        assert_eq!(
            hello_fingerprint(X509_SPKI_BLOB, "ABcd"),
            hello_fingerprint(X509_SPKI_BLOB, "abCD")
        );
        assert_ne!(hello_fingerprint(0, "abcd"), hello_fingerprint(1, "abcd"));
    }

    #[test]
    fn enrolling_audits_and_pins_the_key() {
        let mut store = Store::open_memory().unwrap();
        store
            .enroll_hello_public_key(X509_SPKI_BLOB, KEY_A, 10, &SEED)
            .unwrap();
        let fingerprint = hello_fingerprint(X509_SPKI_BLOB, KEY_A);
        let enrolled = payloads(&store, "hello.enrolled");
        assert_eq!(enrolled.len(), 1);
        let payload: serde_json::Value = serde_json::from_str(&enrolled[0]).unwrap();
        assert_eq!(payload["fingerprint"], fingerprint);
        assert_eq!(payload["blob_type"], X509_SPKI_BLOB);
        assert!(audit_log::verify(&store, &SEED).unwrap());
        assert_eq!(
            audit_log::read_hello_pin(&store).unwrap(),
            Some(fingerprint)
        );
        assert!(pinned_public_key(&store).unwrap().is_some());
        // The same key again is accepted without a second audit row.
        store
            .enroll_hello_public_key(X509_SPKI_BLOB, &KEY_A.to_uppercase(), 20, &SEED)
            .unwrap();
        assert_eq!(payloads(&store, "hello.enrolled").len(), 1);
    }

    #[test]
    fn a_second_key_cannot_replace_the_enrolled_one() {
        let mut store = Store::open_memory().unwrap();
        store
            .enroll_hello_public_key(X509_SPKI_BLOB, KEY_A, 10, &SEED)
            .unwrap();
        let err = store
            .enroll_hello_public_key(X509_SPKI_BLOB, KEY_B, 20, &SEED)
            .unwrap_err();
        assert!(err.to_string().contains("pinned fingerprint"), "{err}");
        assert_eq!(
            store.hello_public_key().unwrap(),
            Some((X509_SPKI_BLOB, KEY_A.to_string()))
        );
        assert_eq!(payloads(&store, "hello.enrolled").len(), 1);
    }

    #[test]
    fn deleting_the_row_does_not_free_the_pin() {
        let mut store = Store::open_memory().unwrap();
        store
            .enroll_hello_public_key(X509_SPKI_BLOB, KEY_A, 10, &SEED)
            .unwrap();
        store
            .connection()
            .execute("DELETE FROM hello_public_key", [])
            .unwrap();
        let err = store
            .enroll_hello_public_key(X509_SPKI_BLOB, KEY_B, 20, &SEED)
            .unwrap_err();
        assert!(err.to_string().contains("pinned fingerprint"), "{err}");
        assert!(store.hello_public_key().unwrap().is_none());
        // A row planted straight into SQL does not match the pin.
        plant_row(&store, KEY_B);
        assert!(pinned_public_key(&store).unwrap().is_none());
        assert!(!verify_stored(&store, "message", &"00".repeat(256)).unwrap());
    }

    #[test]
    fn an_unpinned_row_does_not_verify() {
        let store = Store::open_memory().unwrap();
        plant_row(&store, KEY_A);
        assert!(pinned_public_key(&store).unwrap().is_none());
        assert!(!verify_stored(&store, "message", &"00".repeat(256)).unwrap());
    }

    #[test]
    fn a_legacy_row_is_pinned_by_enrolling_the_same_key_again() {
        let mut store = Store::open_memory().unwrap();
        plant_row(&store, KEY_A);
        let err = store
            .enroll_hello_public_key(X509_SPKI_BLOB, KEY_B, 10, &SEED)
            .unwrap_err();
        assert!(err.to_string().contains("pinned fingerprint"), "{err}");
        store
            .enroll_hello_public_key(X509_SPKI_BLOB, KEY_A, 10, &SEED)
            .unwrap();
        assert_eq!(
            audit_log::read_hello_pin(&store).unwrap(),
            Some(hello_fingerprint(X509_SPKI_BLOB, KEY_A))
        );
        assert!(pinned_public_key(&store).unwrap().is_some());
    }

    #[test]
    fn the_pin_survives_appends_and_a_reopen() {
        let data = temp_data("hello-pin");
        let fingerprint = hello_fingerprint(X509_SPKI_BLOB, KEY_A);
        {
            let mut store = Store::open(&data).unwrap();
            audit_log::append(&mut store, "before", "x", 5, &SEED).unwrap();
            store
                .enroll_hello_public_key(X509_SPKI_BLOB, KEY_A, 10, &SEED)
                .unwrap();
            audit_log::append(&mut store, "after", "y", 20, &SEED).unwrap();
            audit_log::append(&mut store, "later", "z", 30, &SEED).unwrap();
            assert_eq!(
                audit_log::read_hello_pin(&store).unwrap(),
                Some(fingerprint.clone())
            );
        }
        let sidecar = audit_log::read_sidecar(&crate::audit_tip_path(&data)).unwrap();
        assert_eq!(sidecar.hello_pin, Some(fingerprint.clone()));
        let mut store = Store::open(&data).unwrap();
        assert_eq!(
            audit_log::read_hello_pin(&store).unwrap(),
            Some(fingerprint)
        );
        assert!(audit_log::verify(&store, &SEED).unwrap());
        assert!(pinned_public_key(&store).unwrap().is_some());
        let err = store
            .enroll_hello_public_key(X509_SPKI_BLOB, KEY_B, 40, &SEED)
            .unwrap_err();
        assert!(err.to_string().contains("pinned fingerprint"), "{err}");
    }

    #[test]
    fn a_reset_is_audited_first_and_keeps_the_tip() {
        let data = temp_data("hello-reset");
        let mut store = Store::open(&data).unwrap();
        store
            .enroll_hello_public_key(X509_SPKI_BLOB, KEY_A, 10, &SEED)
            .unwrap();
        let err = store.reset_hello_enrollment("  ", 20, &SEED).unwrap_err();
        assert!(err.to_string().contains("reason"), "{err}");
        assert!(payloads(&store, "hello.reset").is_empty());
        store
            .reset_hello_enrollment("lost device", 30, &SEED)
            .unwrap();
        let reset = payloads(&store, "hello.reset");
        let payload: serde_json::Value = serde_json::from_str(&reset[0]).unwrap();
        assert_eq!(
            payload["fingerprint"],
            hello_fingerprint(X509_SPKI_BLOB, KEY_A)
        );
        assert_eq!(payload["reason"], "lost device");
        let sidecar = audit_log::read_sidecar(&crate::audit_tip_path(&data)).unwrap();
        assert!(sidecar.tip.is_some());
        assert!(sidecar.hello_pin.is_none());
        store
            .enroll_hello_public_key(X509_SPKI_BLOB, KEY_B, 40, &SEED)
            .unwrap();
        assert!(audit_log::verify(&store, &SEED).unwrap());
        drop(store);
        Store::open(&data).unwrap();
    }
}

/// Manual H1 check on a real Windows PC with Hello. See docs/hello-hardware-test.md.
/// CI has no Hello hardware, so this is ignored and has not been run.
#[cfg(all(test, windows))]
mod hardware {
    use super::*;

    #[test]
    #[ignore = "needs a Windows PC with Windows Hello and an interactive session"]
    fn hello_hardware_sign_and_verify() {
        let (blob_type, spki_hex) = hello_public_key_material().expect("Hello key");
        assert_eq!(blob_type, X509_SPKI_BLOB);
        let spki = crate::signature::decode_hex_bytes(&spki_hex).expect("hex");
        println!(
            "spki_len={} fingerprint={}",
            spki.len(),
            hello_fingerprint(blob_type, &spki_hex)
        );
        let message = "v1\nhardware-test\napprove\n\ncard\nnonce\n1\nhash\n";
        let prompt = consent_prompt("approve", "post_pr_comment", "DasVR/NIL");
        let signature_hex = sign_approval_message(message, &prompt).expect("Hello signature");
        let signature = crate::signature::decode_hex_bytes(&signature_hex).expect("hex");
        println!("signature_len={}", signature.len());
        assert_eq!(signature.len(), 256, "RSA-2048 signature");
        assert!(cng::verify(
            blob_type,
            &spki,
            message.as_bytes(),
            &signature
        ));
        assert!(!cng::verify(blob_type, &spki, b"tampered", &signature));
        let mut flipped = signature.clone();
        flipped[10] ^= 1;
        assert!(!cng::verify(blob_type, &spki, message.as_bytes(), &flipped));
        let mut other = spki.clone();
        let last = other.len() - 5;
        other[last] ^= 1;
        assert!(!cng::verify(
            blob_type,
            &other,
            message.as_bytes(),
            &signature
        ));
        let mut store = Store::open_memory().expect("store");
        store
            .enroll_hello_public_key(blob_type, &spki_hex, 1, &[7u8; 32])
            .expect("enroll");
        let other_hex = crate::signature::hex_encode(&other);
        assert!(store
            .enroll_hello_public_key(blob_type, &other_hex, 2, &[7u8; 32])
            .is_err());
        assert!(verify_stored(&store, message, &signature_hex).expect("verify"));
    }
}
