//! Native sight signal for the seen lock (SD LOW 6).
//!
//! WebView2 is not guaranteed to fire `blur` or `visibilitychange` when the
//! host window is minimized or loses focus to another window. The shell
//! forwards those window events to the page as a
//! `dasdevbot:native-sight` CustomEvent with `detail.visible`. The approval
//! card treats `visible: false` like a window blur: the seen lock and any hold
//! reset, and the dwell starts from zero when the window has focus again.
//!
//! Not verified on real Windows. Occlusion by a window that does not take
//! focus is not reported by Tauri, so it is not covered here.

/// The page event name. `ApprovalCard.svelte` listens for it.
pub(crate) const SIGHT_EVENT: &str = "dasdevbot:native-sight";

/// Windows whose page can show an approval card.
const SIGHTED: [&str; 2] = ["main", "card"];

/// What a window event means for sight, if anything.
pub(crate) fn sight_for(focused: Option<bool>, minimized: Option<bool>) -> Option<bool> {
    if minimized == Some(true) {
        return Some(false);
    }
    focused
}

/// The script that delivers one sight change to the page.
pub(crate) fn sight_script(visible: bool) -> String {
    format!(
        "window.dispatchEvent(new CustomEvent({SIGHT_EVENT:?},{{detail:{{visible:{visible}}}}}))"
    )
}

pub(crate) fn forward(window: &tauri::Window, event: &tauri::WindowEvent) {
    use tauri::Manager;
    if !SIGHTED.contains(&window.label()) {
        return;
    }
    let visible = match event {
        tauri::WindowEvent::Focused(focused) => sight_for(Some(*focused), None),
        tauri::WindowEvent::Resized(_) => sight_for(None, window.is_minimized().ok()),
        _ => None,
    };
    let Some(visible) = visible else {
        return;
    };
    if let Some(webview) = window.app_handle().get_webview_window(window.label()) {
        let _ = webview.eval(sight_script(visible));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimize_and_blur_drop_sight_and_focus_restores_it() {
        assert_eq!(sight_for(None, Some(true)), Some(false));
        assert_eq!(sight_for(Some(true), Some(true)), Some(false));
        assert_eq!(sight_for(Some(false), None), Some(false));
        assert_eq!(sight_for(Some(true), None), Some(true));
        assert_eq!(sight_for(None, Some(false)), None);
        assert_eq!(sight_for(None, None), None);
    }

    #[test]
    fn the_script_dispatches_the_card_event() {
        assert_eq!(
            sight_script(false),
            "window.dispatchEvent(new CustomEvent(\"dasdevbot:native-sight\",{detail:{visible:false}}))"
        );
        assert!(sight_script(true).contains("visible:true"));
    }
}
