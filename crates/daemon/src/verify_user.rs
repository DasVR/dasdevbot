//! User verification for card decisions. Production signing asks Windows Hello.
//! Other targets refuse, so a signature cannot be minted without that check.

use std::fmt;

pub trait UserVerifier: Send + Sync {
    fn verify_user(&self, prompt: &str) -> std::result::Result<(), UserVerifyError>;
}

#[derive(Debug)]
pub enum UserVerifyError {
    Denied,
    Unavailable(String),
}

impl fmt::Display for UserVerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UserVerifyError::Denied => write!(f, "user verification was denied"),
            UserVerifyError::Unavailable(text) => write!(f, "{text}"),
        }
    }
}

pub struct PlatformVerifier;

impl UserVerifier for PlatformVerifier {
    fn verify_user(&self, prompt: &str) -> std::result::Result<(), UserVerifyError> {
        platform_verify(prompt)
    }
}

/// True only when the binary's production verifier is Windows Hello.
pub fn user_verification_is_real() -> bool {
    cfg!(windows)
}

#[cfg(windows)]
fn platform_verify(prompt: &str) -> std::result::Result<(), UserVerifyError> {
    windows_hello::verify(prompt)
}

#[cfg(not(windows))]
fn platform_verify(_prompt: &str) -> std::result::Result<(), UserVerifyError> {
    Err(UserVerifyError::Unavailable(
        "user verification requires Windows Hello".into(),
    ))
}

#[cfg(windows)]
mod windows_hello {
    use super::UserVerifyError;
    use windows::core::HSTRING;
    use windows::Security::Credentials::UI::{
        UserConsentVerificationResult, UserConsentVerifier, UserConsentVerifierAvailability,
    };

    pub fn verify(prompt: &str) -> std::result::Result<(), UserVerifyError> {
        let availability = UserConsentVerifier::CheckAvailabilityAsync()
            .map_err(|err| UserVerifyError::Unavailable(err.to_string()))?
            .get()
            .map_err(|err| UserVerifyError::Unavailable(err.to_string()))?;
        if availability != UserConsentVerifierAvailability::Available {
            return Err(UserVerifyError::Unavailable(
                "Windows Hello is not available".into(),
            ));
        }
        let result = UserConsentVerifier::RequestVerificationAsync(&HSTRING::from(prompt))
            .map_err(|err| UserVerifyError::Unavailable(err.to_string()))?
            .get()
            .map_err(|err| UserVerifyError::Unavailable(err.to_string()))?;
        if result == UserConsentVerificationResult::Verified {
            Ok(())
        } else {
            Err(UserVerifyError::Denied)
        }
    }
}

#[cfg(test)]
pub struct TestVerifier {
    pub allow: bool,
}

#[cfg(test)]
impl UserVerifier for TestVerifier {
    fn verify_user(&self, _prompt: &str) -> std::result::Result<(), UserVerifyError> {
        if self.allow {
            Ok(())
        } else {
            Err(UserVerifyError::Denied)
        }
    }
}
