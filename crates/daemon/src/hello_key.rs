//! External-tier approvals. The private key is a non-exportable Windows Hello
//! key created with `KeyCredentialManager`. The daemon stores only the public
//! key and verifies with CNG. Other targets have no such key, so the external
//! tier stays denied. Internal tiers keep the Phase 1 Ed25519 path.

use crate::store::Store;
use crate::verify_user::user_verification_is_real;
use crate::Result;

#[cfg_attr(not(windows), allow(dead_code))]
pub const HELLO_KEY_NAME: &str = "dasdevbot-approval";
/// `CryptographicPublicKeyBlobType::BCryptEccFullPublicKey`.
#[cfg_attr(not(windows), allow(dead_code))]
pub const BCRYPT_ECC_FULL_PUBLIC: i64 = 4;

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

pub fn verify_stored(store: &Store, message: &str, signature_hex: &str) -> Result<bool> {
    if !signing_path_is_present() {
        return Ok(false);
    }
    let Some((blob_type, public_key)) = store.hello_public_key()? else {
        return Ok(false);
    };
    let Some(public_key) = crate::signature::decode_hex_bytes(&public_key) else {
        return Ok(false);
    };
    let Some(signature) = crate::signature::decode_hex_bytes(signature_hex) else {
        return Ok(false);
    };
    Ok(verify_cng(blob_type, &public_key, message.as_bytes(), &signature))
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
    Ok((BCRYPT_ECC_FULL_PUBLIC, crate::signature::hex_encode(&bytes)))
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
            .RetrievePublicKeyWithBlobType(CryptographicPublicKeyBlobType::BCryptEccFullPublicKey)
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

    fn buffer_bytes(
        buffer: &windows::Storage::Streams::IBuffer,
    ) -> Result<Vec<u8>, String> {
        let reader = DataReader::FromBuffer(buffer).map_err(|err| err.to_string())?;
        let len = reader
            .UnconsumedBufferLength()
            .map_err(|err| err.to_string())? as usize;
        let mut bytes = vec![0u8; len];
        reader.ReadBytes(&mut bytes).map_err(|err| err.to_string())?;
        Ok(bytes)
    }
}

#[cfg(windows)]
mod cng {
    use sha2::Digest;
    use windows::Win32::Security::Cryptography::{
        BCryptCloseAlgorithmProvider, BCryptDestroyKey, BCryptImportKeyPair,
        BCryptOpenAlgorithmProvider, BCryptVerifySignature, BCRYPT_ALG_HANDLE,
        BCRYPT_ECCFULLPUBLIC_BLOB, BCRYPT_ECDSA_P256_ALGORITHM, BCRYPT_FLAGS, BCRYPT_KEY_HANDLE,
        BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS,
    };

    pub fn verify(blob_type: i64, public_key: &[u8], message: &[u8], signature: &[u8]) -> bool {
        if blob_type != super::BCRYPT_ECC_FULL_PUBLIC {
            return false;
        }
        let digest = sha2::Sha256::digest(message);
        unsafe { verify_ecdsa(public_key, digest.as_slice(), signature) }
    }

    unsafe fn verify_ecdsa(public_key: &[u8], digest: &[u8], signature: &[u8]) -> bool {
        let mut algorithm = BCRYPT_ALG_HANDLE(std::ptr::null_mut());
        let opened = unsafe {
            BCryptOpenAlgorithmProvider(
                &mut algorithm,
                BCRYPT_ECDSA_P256_ALGORITHM,
                windows::core::PCWSTR::null(),
                BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS(0),
            )
        };
        if opened.0 != 0 {
            return false;
        }
        let mut key = BCRYPT_KEY_HANDLE(std::ptr::null_mut());
        let imported = unsafe {
            BCryptImportKeyPair(
                algorithm,
                None,
                BCRYPT_ECCFULLPUBLIC_BLOB,
                &mut key,
                public_key,
                0,
            )
        };
        if imported.0 != 0 {
            unsafe {
                let _ = BCryptCloseAlgorithmProvider(algorithm, 0);
            }
            return false;
        }
        let status = unsafe {
            BCryptVerifySignature(key, None, digest, signature, BCRYPT_FLAGS(0))
        };
        unsafe {
            let _ = BCryptDestroyKey(key);
            let _ = BCryptCloseAlgorithmProvider(algorithm, 0);
        }
        status.0 == 0
    }
}
