//! Grant expiry. Defaults may be shortened and must not be lengthened.
//! Destructive grants are not issued in phase 1.

use crate::EffectClass;

pub const WRITE_LOCAL_EXPIRY_MS: u64 = 24 * 60 * 60 * 1000;
pub const EXTERNAL_EXPIRY_MS: u64 = 4 * 60 * 60 * 1000;
pub const READ_EXPIRY_MS: u64 = 30 * 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantError {
    DestructiveDenied,
    LengthensExpiry,
    Empty,
}

pub fn default_expiry_ms(class: EffectClass) -> Result<u64, GrantError> {
    match class {
        EffectClass::Read => Ok(READ_EXPIRY_MS),
        EffectClass::WriteLocal => Ok(WRITE_LOCAL_EXPIRY_MS),
        EffectClass::External => Ok(EXTERNAL_EXPIRY_MS),
        EffectClass::Destructive => Err(GrantError::DestructiveDenied),
    }
}

/// `shorten_to_ms` replaces the tier default only when it is shorter.
pub fn issue_expiry(
    class: EffectClass,
    now_ms: u64,
    shorten_to_ms: Option<u64>,
) -> Result<u64, GrantError> {
    let default_ms = default_expiry_ms(class)?;
    let ttl = match shorten_to_ms {
        None => default_ms,
        Some(0) => return Err(GrantError::Empty),
        Some(ms) if ms <= default_ms => ms,
        Some(_) => return Err(GrantError::LengthensExpiry),
    };
    Ok(now_ms.saturating_add(ttl))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grants_may_shorten_expiry_and_must_not_lengthen_it() {
        let now = 1_000;
        let full = issue_expiry(EffectClass::External, now, None).unwrap();
        assert_eq!(full, now + EXTERNAL_EXPIRY_MS);
        let shorter = issue_expiry(EffectClass::External, now, Some(60_000)).unwrap();
        assert_eq!(shorter, now + 60_000);
        assert_eq!(
            issue_expiry(EffectClass::External, now, Some(EXTERNAL_EXPIRY_MS + 1)),
            Err(GrantError::LengthensExpiry)
        );
        assert_eq!(
            issue_expiry(EffectClass::Destructive, now, Some(1_000)),
            Err(GrantError::DestructiveDenied)
        );
        assert_eq!(
            issue_expiry(EffectClass::WriteLocal, now, Some(0)),
            Err(GrantError::Empty)
        );
    }
}
