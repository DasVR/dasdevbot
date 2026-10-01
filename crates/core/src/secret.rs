//! Redacted secret values. The daemon store and its CLI live with the secret
//! store this branch rebases onto.

use std::fmt;

use zeroize::Zeroize;

/// Owns a secret value. Debug output is redacted. Drop zeroes the bytes.
pub struct SecretValue(String);

impl SecretValue {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// Last four characters. A value of four or fewer is fully masked.
    pub fn last4(&self) -> String {
        let chars: Vec<char> = self.0.chars().collect();
        if chars.len() <= 4 {
            return "••••".into();
        }
        chars.iter().skip(chars.len() - 4).collect()
    }

    /// For a keychain write. Do not log, format, or put this in an error.
    pub fn reveal_for_keychain(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretValue([redacted])")
    }
}

impl Drop for SecretValue {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_the_secret_and_a_short_value_is_fully_masked() {
        let value = SecretValue::new("super-secret-value-wxyz".into());
        let rendered = format!("{value:?}");
        assert_eq!(rendered, "SecretValue([redacted])");
        assert!(!rendered.contains("super-secret"));
        assert_eq!(value.last4(), "wxyz");
        let short = SecretValue::new("abcd".into());
        assert_eq!(short.last4(), "••••");
        assert!(!short.last4().contains("abcd"));
    }
}
