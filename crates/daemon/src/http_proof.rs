//! The daemon proves itself to the Tauri shell on 127.0.0.1:8787 (#36 M2).
//!
//! The bearer authenticates the shell to the daemon. This is the other
//! direction: before the shell sends the bearer, and on every response it
//! trusts, the daemon shows it knows the per-launch bearer without revealing
//! it. The shell sends a fresh random challenge in `X-Dasdevbot-Challenge`.
//! The daemon answers with `X-Dasdevbot-Proof`, a keyed blake3 hash over the
//! challenge, the request line, the status and the exact response body. The key
//! is derived from the bearer.
//!
//! A process that took port 8787 while the daemon was down, under another user
//! account, cannot read `<data>.token` (mode 0600, owner-only DACL on Windows),
//! so it cannot forge a proof and never receives the bearer. A same-user
//! process can read the token file and is out of scope, as on the shell socket.

use subtle::ConstantTimeEq;

pub const CHALLENGE_HEADER: &str = "X-Dasdevbot-Challenge";
pub const PROOF_HEADER: &str = "X-Dasdevbot-Proof";

const CONTEXT: &str = "dasdevbot 2026-10-08 loopback http daemon proof v1";

/// 32 random bytes as 64 lowercase hex characters.
pub fn new_challenge() -> String {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).expect("os rng");
    crate::hex_encode(&bytes)
}

/// Only a 64-character lowercase hex challenge is answered.
pub fn valid_challenge(challenge: &str) -> bool {
    challenge.len() == 64
        && challenge
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// `request_line` is `"<METHOD> <path>"` with no query string, e.g. `"GET /v1/snapshot"`.
pub fn proof(token: &str, challenge: &str, request_line: &str, status: u16, body: &[u8]) -> String {
    let key = blake3::derive_key(CONTEXT, token.as_bytes());
    let mut hasher = blake3::Hasher::new_keyed(&key);
    hasher.update(challenge.as_bytes());
    hasher.update(b"\n");
    hasher.update(request_line.as_bytes());
    hasher.update(b"\n");
    hasher.update(status.to_string().as_bytes());
    hasher.update(b"\n");
    hasher.update(body);
    hasher.finalize().to_hex().to_string()
}

/// Constant-time check of a presented proof. A missing or malformed challenge fails.
pub fn verify(
    token: &str,
    challenge: &str,
    request_line: &str,
    status: u16,
    body: &[u8],
    presented: Option<&str>,
) -> bool {
    let Some(presented) = presented else {
        return false;
    };
    if !valid_challenge(challenge) {
        return false;
    }
    let expected = proof(token, challenge, request_line, status, body);
    let presented = presented.trim().as_bytes();
    expected.len() == presented.len() && bool::from(expected.as_bytes().ct_eq(presented))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn a_proof_verifies_only_for_the_same_token_challenge_line_status_and_body() {
        let challenge = new_challenge();
        assert!(valid_challenge(&challenge));
        let line = "GET /v1/snapshot";
        let good = proof(TOKEN, &challenge, line, 200, b"{}");
        assert!(verify(TOKEN, &challenge, line, 200, b"{}", Some(&good)));
        assert!(!verify(
            "another-token-another-token-0000",
            &challenge,
            line,
            200,
            b"{}",
            Some(&good)
        ));
        assert!(!verify(
            TOKEN,
            &new_challenge(),
            line,
            200,
            b"{}",
            Some(&good)
        ));
        assert!(!verify(
            TOKEN,
            &challenge,
            "POST /v1/events",
            200,
            b"{}",
            Some(&good)
        ));
        assert!(!verify(TOKEN, &challenge, line, 500, b"{}", Some(&good)));
        assert!(!verify(
            TOKEN,
            &challenge,
            line,
            200,
            b"{\"a\":1}",
            Some(&good)
        ));
        assert!(!verify(TOKEN, &challenge, line, 200, b"{}", None));
        assert!(!verify(TOKEN, &challenge, line, 200, b"{}", Some("")));
    }

    #[test]
    fn the_proof_does_not_contain_the_token() {
        let challenge = new_challenge();
        let made = proof(TOKEN, &challenge, "GET /v1/health", 200, b"{}");
        assert!(!made.contains(TOKEN));
        assert_eq!(made.len(), 64);
    }

    #[test]
    fn only_lowercase_hex_challenges_of_64_chars_are_valid() {
        assert!(!valid_challenge(""));
        assert!(!valid_challenge(&"a".repeat(63)));
        assert!(!valid_challenge(&"A".repeat(64)));
        assert!(!valid_challenge(&"g".repeat(64)));
        assert!(valid_challenge(&"0".repeat(64)));
    }
}
