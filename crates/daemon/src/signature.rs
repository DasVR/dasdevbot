//! Decision signatures for internal tiers. The approval seed is read from the
//! secret store here and is never an IPC argument. External tiers do not use
//! this seed: the daemon verifies a Windows Hello public key and does not sign
//! with the same material. Callers pass a [`SecretHandle`], never the raw seed.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier};

use crate::secrets::{SecretHandle, APPROVAL_KEY_NAME};

pub const NONCE_TTL_MS: u64 = 60_000;
pub const DECISION_PURPOSE: &str = "decision";
pub const UNDO_PURPOSE: &str = "undo";

pub fn decision_message(
    approval_id: &str,
    decision: &str,
    reason: &str,
    window: &str,
    nonce: &str,
    fencing: u64,
    action_hash: &str,
) -> String {
    format!(
        "v1\n{approval_id}\n{decision}\n{reason}\n{window}\n{nonce}\n{fencing}\n{action_hash}\n"
    )
}

pub fn undo_message(
    approval_id: &str,
    status: &str,
    window: &str,
    nonce: &str,
    fencing: u64,
    action_hash: &str,
) -> String {
    format!("v1-undo\n{approval_id}\n{status}\n{window}\n{nonce}\n{fencing}\n{action_hash}\n")
}

pub fn action_hash(effect_class: &str, action: &str, draft: &str) -> String {
    blake3::hash(format!("{effect_class}\n{action}\n{draft}").as_bytes())
        .to_hex()
        .to_string()
}

/// Bound to the card, the purpose, and the exact payload being signed.
pub fn payload_hash(
    purpose: &str,
    approval_id: &str,
    decision: &str,
    reason: &str,
    window: &str,
    action_hash: &str,
) -> String {
    blake3::hash(
        format!("{purpose}\n{approval_id}\n{decision}\n{reason}\n{window}\n{action_hash}")
            .as_bytes(),
    )
    .to_hex()
    .to_string()
}

/// Verify with the approval key from the secret store. A missing key is a failed check.
pub fn verify_decision_signature(
    secrets: &dyn SecretHandle,
    message: &str,
    signature_hex: &str,
) -> bool {
    let Ok(Some(secret)) = secrets.get(APPROVAL_KEY_NAME) else {
        return false;
    };
    let Some(seed) = decode_seed(secret.expose()) else {
        return false;
    };
    let Some(bytes) = decode_hex(signature_hex) else {
        return false;
    };
    let Ok(bytes) = <[u8; 64]>::try_from(bytes) else {
        return false;
    };
    let Ok(signature) = Signature::from_slice(&bytes) else {
        return false;
    };
    let key = SigningKey::from_bytes(&seed);
    Verifier::verify(&key, message.as_bytes(), &signature).is_ok()
}

pub fn sign_with_approval_key(secrets: &dyn SecretHandle, message: &str) -> Option<String> {
    let secret = secrets.get(APPROVAL_KEY_NAME).ok()??;
    let seed = decode_seed(secret.expose())?;
    let key = SigningKey::from_bytes(&seed);
    Some(hex_encode(&key.sign(message.as_bytes()).to_bytes()))
}

pub fn sign_bytes(seed: &[u8; 32], message: &[u8]) -> String {
    let key = SigningKey::from_bytes(seed);
    hex_encode(&key.sign(message).to_bytes())
}

pub fn verify_bytes(seed: &[u8; 32], message: &[u8], signature_hex: &str) -> bool {
    let Some(bytes) = decode_hex(signature_hex) else {
        return false;
    };
    let Ok(bytes) = <[u8; 64]>::try_from(bytes) else {
        return false;
    };
    let Ok(signature) = Signature::from_slice(&bytes) else {
        return false;
    };
    let key = SigningKey::from_bytes(seed);
    Verifier::verify(&key, message, &signature).is_ok()
}

pub fn encode_seed(seed: &[u8; 32]) -> String {
    hex_encode(seed)
}

pub fn decode_seed(text: &str) -> Option<[u8; 32]> {
    let text = text.trim();
    if text.len() == 32 && !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return text.as_bytes().try_into().ok();
    }
    let bytes = decode_hex(text)?;
    <[u8; 32]>::try_from(bytes).ok()
}

pub fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

pub(crate) fn decode_hex_bytes(text: &str) -> Option<Vec<u8>> {
    decode_hex(text)
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    let mut index = 0;
    while index < bytes.len() {
        let hi = from_hex(bytes[index])?;
        let lo = from_hex(bytes[index + 1])?;
        out.push((hi << 4) | lo);
        index += 2;
    }
    Some(out)
}

fn from_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
