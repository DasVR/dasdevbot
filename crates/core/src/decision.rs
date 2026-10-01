//! Approval decisions. Phase 1 accepts them only from the card window over
//! Tauri IPC. HTTP, voice, and destructive actions are denied.

use crate::EffectClass;

pub const CARD_WINDOW: &str = "card";
pub const MAIN_WINDOW: &str = "main";
pub const SETTINGS_WINDOW: &str = "settings";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    TauriIpc,
    Http,
    Voice,
    Cli,
    Banner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionDeny {
    Http,
    Voice,
    WrongWindow,
    Destructive,
    /// The external tier is off in Phase 1. See `EXTERNAL_TIER_ENABLED`.
    External,
    NotASurface,
}

pub fn authorize_decision(
    surface: Surface,
    window: &str,
    voice: bool,
    class: EffectClass,
) -> Result<(), DecisionDeny> {
    if voice || matches!(surface, Surface::Voice) {
        return Err(DecisionDeny::Voice);
    }
    if matches!(surface, Surface::Http) {
        return Err(DecisionDeny::Http);
    }
    if matches!(class, EffectClass::Destructive) {
        return Err(DecisionDeny::Destructive);
    }
    if matches!(class, EffectClass::External) && !crate::gate::EXTERNAL_TIER_ENABLED {
        return Err(DecisionDeny::External);
    }
    match surface {
        Surface::TauriIpc if window == CARD_WINDOW => Ok(()),
        Surface::TauriIpc => Err(DecisionDeny::WrongWindow),
        Surface::Http => Err(DecisionDeny::Http),
        Surface::Voice => Err(DecisionDeny::Voice),
        Surface::Cli | Surface::Banner => Err(DecisionDeny::NotASurface),
    }
}

pub fn authorize_secret_window(window: &str) -> Result<(), DecisionDeny> {
    if window == MAIN_WINDOW || window == SETTINGS_WINDOW {
        Ok(())
    } else {
        Err(DecisionDeny::WrongWindow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_voice_and_destructive_cannot_decide() {
        assert_eq!(
            authorize_decision(Surface::Http, CARD_WINDOW, false, EffectClass::External),
            Err(DecisionDeny::Http)
        );
        assert_eq!(
            authorize_decision(Surface::TauriIpc, CARD_WINDOW, true, EffectClass::Read),
            Err(DecisionDeny::Voice)
        );
        assert_eq!(
            authorize_decision(Surface::Voice, CARD_WINDOW, false, EffectClass::External),
            Err(DecisionDeny::Voice)
        );
        assert_eq!(
            authorize_decision(
                Surface::TauriIpc,
                CARD_WINDOW,
                false,
                EffectClass::Destructive
            ),
            Err(DecisionDeny::Destructive)
        );
        assert_eq!(
            authorize_decision(
                Surface::TauriIpc,
                MAIN_WINDOW,
                false,
                EffectClass::WriteLocal
            ),
            Err(DecisionDeny::WrongWindow)
        );
        assert_eq!(
            authorize_decision(Surface::TauriIpc, CARD_WINDOW, false, EffectClass::External),
            Err(DecisionDeny::External)
        );
        assert_eq!(
            authorize_decision(
                Surface::TauriIpc,
                CARD_WINDOW,
                false,
                EffectClass::WriteLocal
            ),
            Ok(())
        );
        assert_eq!(authorize_secret_window(SETTINGS_WINDOW), Ok(()));
        assert_eq!(authorize_secret_window(MAIN_WINDOW), Ok(()));
        assert_eq!(
            authorize_secret_window(CARD_WINDOW),
            Err(DecisionDeny::WrongWindow)
        );
    }
}
