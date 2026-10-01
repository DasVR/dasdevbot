//! Logical window geometry for the three shell forms.
//!
//! Phase 1 is Windows. These sizes are what the Tauri commands apply.
//! Folded in from the former `crates/shell`. This module does not touch a
//! window, so its tests run on Linux CI with the rest of `src-tauri`.

/// Mock full window (`SH.full.win`). The 1280×800 stills are a viewport, not this size.
pub const FULL_WIDTH: f64 = 1360.0;
pub const FULL_HEIGHT: f64 = 828.0;

/// Narrow side window from the shell mock (`SH.comp`).
pub const COMPANION_WIDTH: f64 = 400.0;
pub const COMPANION_HEIGHT: f64 = 790.0;

/// Floating pill. Same box as the mock composer-as-window.
pub const PILL_WIDTH: f64 = 400.0;
pub const PILL_HEIGHT: f64 = 52.0;

/// Mock companion inset from the top of the work area.
pub const COMPANION_TOP: f64 = 72.0;
/// Mock companion inset from the right of the work area (1440 − 1000 − 400).
pub const COMPANION_RIGHT: f64 = 40.0;
/// Mock pill inset from the bottom (900 − 800 − 52).
pub const PILL_BOTTOM: f64 = 48.0;

/// Window corner radius. Matches `--r-md` (14). Kept numeric for the OS window.
pub const WINDOW_RADIUS: f64 = 14.0;
/// Pill corner radius. The mock uses 26, which is not a token.
pub const PILL_RADIUS: f64 = 26.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellForm {
    Full,
    Companion,
    Pill,
}

impl ShellForm {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "full" => Some(Self::Full),
            "companion" => Some(Self::Companion),
            "pill" => Some(Self::Pill),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FormSpec {
    pub width: f64,
    pub height: f64,
    pub radius: f64,
    /// Glass content. Only the pill. Full and companion paint opaque paper.
    pub glass: bool,
    pub always_on_top: bool,
    pub resizable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Monitor {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LogicalRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

pub fn form_spec(form: ShellForm) -> FormSpec {
    match form {
        ShellForm::Full => FormSpec {
            width: FULL_WIDTH,
            height: FULL_HEIGHT,
            radius: WINDOW_RADIUS,
            glass: false,
            always_on_top: false,
            resizable: true,
        },
        ShellForm::Companion => FormSpec {
            width: COMPANION_WIDTH,
            height: COMPANION_HEIGHT,
            radius: WINDOW_RADIUS,
            glass: false,
            always_on_top: false,
            resizable: true,
        },
        ShellForm::Pill => FormSpec {
            width: PILL_WIDTH,
            height: PILL_HEIGHT,
            radius: PILL_RADIUS,
            glass: true,
            always_on_top: true,
            resizable: false,
        },
    }
}

/// Place a form on a monitor in logical pixels. The pill is centered.
/// Companion hugs the right edge. Full is centered. Nothing is placed off-screen.
pub fn place(form: ShellForm, monitor: Monitor) -> LogicalRect {
    let spec = form_spec(form);
    let width = spec.width.min(monitor.width.max(1.0));
    let height = spec.height.min(monitor.height.max(1.0));
    let (x, y) = match form {
        ShellForm::Full => (
            monitor.x + (monitor.width - width) / 2.0,
            monitor.y + (monitor.height - height) / 2.0,
        ),
        ShellForm::Companion => (
            monitor.x + monitor.width - width - COMPANION_RIGHT,
            monitor.y + COMPANION_TOP,
        ),
        ShellForm::Pill => (
            monitor.x + (monitor.width - width) / 2.0,
            monitor.y + monitor.height - height - PILL_BOTTOM,
        ),
    };
    let max_x = monitor.x + monitor.width - width;
    let max_y = monitor.y + monitor.height - height;
    LogicalRect {
        x: x.clamp(monitor.x, max_x.max(monitor.x)),
        y: y.clamp(monitor.y, max_y.max(monitor.y)),
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desktop() -> Monitor {
        Monitor {
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        }
    }

    #[test]
    fn specs_match_the_three_forms() {
        let full = form_spec(ShellForm::Full);
        assert_eq!((full.width, full.height), (1360.0, 828.0));
        assert!(!full.glass);
        assert!(full.resizable);
        assert!(!full.always_on_top);

        let companion = form_spec(ShellForm::Companion);
        assert_eq!((companion.width, companion.height), (400.0, 790.0));
        assert!(!companion.glass);

        let pill = form_spec(ShellForm::Pill);
        assert_eq!((pill.width, pill.height), (400.0, 52.0));
        assert!(pill.glass);
        assert!(pill.always_on_top);
        assert!(!pill.resizable);
        assert_eq!(pill.radius, PILL_RADIUS);
    }

    #[test]
    fn pill_is_centered_and_companion_keeps_the_right_margin() {
        let monitor = desktop();
        let pill = place(ShellForm::Pill, monitor);
        assert_eq!(pill.x, (1920.0 - 400.0) / 2.0);
        assert_eq!(pill.y, 1080.0 - 52.0 - PILL_BOTTOM);

        let companion = place(ShellForm::Companion, monitor);
        assert_eq!(companion.x, 1920.0 - 400.0 - COMPANION_RIGHT);
        assert_eq!(companion.y, COMPANION_TOP);
    }

    #[test]
    fn a_short_monitor_clamps_the_companion_instead_of_leaving_the_screen() {
        let monitor = Monitor {
            x: 10.0,
            y: 20.0,
            width: 500.0,
            height: 600.0,
        };
        let companion = place(ShellForm::Companion, monitor);
        assert!(companion.y >= monitor.y);
        assert!(companion.y + companion.height <= monitor.y + monitor.height + 0.5);
        assert!(companion.x >= monitor.x);
        assert!(companion.width <= monitor.width);
    }

    #[test]
    fn unknown_form_names_do_not_parse() {
        assert_eq!(ShellForm::parse("full"), Some(ShellForm::Full));
        assert_eq!(ShellForm::parse("companion"), Some(ShellForm::Companion));
        assert_eq!(ShellForm::parse("pill"), Some(ShellForm::Pill));
        assert_eq!(ShellForm::parse("island"), None);
        assert_eq!(ShellForm::parse("menu-bar"), None);
    }
}
