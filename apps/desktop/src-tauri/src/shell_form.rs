//! Shell-form window commands from #24: prepare, metrics, bounds, and the
//! caption buttons. Geometry is `crate::geometry` (the former `crates/shell`).
//! None of these commands decide anything. Decisions stay on the card window
//! (`sign_decision` / `undo_decision`).

use crate::geometry::{form_spec, place, FormSpec, Monitor, ShellForm};
use tauri::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize, WebviewWindow};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ShellTarget {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    radius: f64,
    glass: bool,
    always_on_top: bool,
    resizable: bool,
}

/// The full form's narrowest native size. Below this width the Approve row
/// would clip, so the page switches to the companion (`FULL_MIN_WIDTH` in
/// geometry.ts). `tauri.conf.json` launches the full form with the same minimum.
pub(crate) const FULL_MIN_WIDTH: f64 = 700.0;
pub(crate) const FULL_MIN_HEIGHT: f64 = 480.0;

/// The minimum size for one bounds update. Companion and pill have none. The
/// full minimum applies only once the morph has reached it, so the tween from
/// a smaller form is never snapped.
pub(crate) fn min_size(form: ShellForm, width: f64, height: f64) -> Option<(f64, f64)> {
    match form {
        ShellForm::Full if width >= FULL_MIN_WIDTH && height >= FULL_MIN_HEIGHT => {
            Some((FULL_MIN_WIDTH, FULL_MIN_HEIGHT))
        }
        _ => None,
    }
}

fn apply_min(window: &WebviewWindow, min: Option<(f64, f64)>) -> Result<(), String> {
    window
        .set_min_size(min.map(|(width, height)| LogicalSize::new(width, height)))
        .map_err(|err| err.to_string())
}

fn parse_form(value: &str) -> Result<ShellForm, String> {
    ShellForm::parse(value).ok_or_else(|| format!("unknown shell form {value}"))
}

fn monitor_of(window: &WebviewWindow) -> Result<Monitor, String> {
    let Some(monitor) = window.current_monitor().map_err(|err| err.to_string())? else {
        return Ok(Monitor {
            x: 0.0,
            y: 0.0,
            width: 1280.0,
            height: 800.0,
        });
    };
    let scale = monitor.scale_factor();
    let size = monitor.size();
    let position = monitor.position();
    Ok(Monitor {
        x: position.x as f64 / scale,
        y: position.y as f64 / scale,
        width: size.width as f64 / scale,
        height: size.height as f64 / scale,
    })
}

fn target_from(form: ShellForm, spec: FormSpec, monitor: Monitor) -> ShellTarget {
    let rect = place(form, monitor);
    ShellTarget {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
        radius: spec.radius,
        glass: spec.glass,
        always_on_top: spec.always_on_top,
        resizable: spec.resizable,
    }
}

fn apply_chrome(window: &WebviewWindow, spec: &FormSpec) -> Result<(), String> {
    window
        .set_resizable(spec.resizable)
        .map_err(|err| err.to_string())?;
    window
        .set_always_on_top(spec.always_on_top)
        .map_err(|err| err.to_string())?;
    // Full and companion are opaque paper. The pill drops the OS shadow so
    // the CSS float shadow is the only one. On Linux this is a no-op when
    // the compositor ignores it.
    let _ = window.set_shadow(!spec.glass);
    Ok(())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ShellMetrics {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

fn logical_pos(
    window: &WebviewWindow,
    position: PhysicalPosition<i32>,
) -> Result<(f64, f64), String> {
    let scale = window.scale_factor().map_err(|err| err.to_string())?;
    Ok((position.x as f64 / scale, position.y as f64 / scale))
}

fn logical_size(window: &WebviewWindow, size: PhysicalSize<u32>) -> Result<(f64, f64), String> {
    let scale = window.scale_factor().map_err(|err| err.to_string())?;
    Ok((size.width as f64 / scale, size.height as f64 / scale))
}

/// Where this form should sit on the current monitor, plus the chrome flags.
#[tauri::command]
pub(crate) fn prepare_shell_form(
    window: WebviewWindow,
    form: String,
) -> Result<ShellTarget, String> {
    let form = parse_form(&form)?;
    let spec = form_spec(form);
    let monitor = monitor_of(&window)?;
    Ok(target_from(form, spec, monitor))
}

/// Current outer bounds in logical pixels. The morph tweens from here.
#[tauri::command]
pub(crate) fn shell_metrics(window: WebviewWindow) -> Result<ShellMetrics, String> {
    let position = window.outer_position().map_err(|err| err.to_string())?;
    let size = window.outer_size().map_err(|err| err.to_string())?;
    let (x, y) = logical_pos(&window, position)?;
    let (width, height) = logical_size(&window, size)?;
    Ok(ShellMetrics {
        x,
        y,
        width,
        height,
    })
}

/// One frame of the morph, or the resting bounds when motion is reduced.
#[tauri::command]
pub(crate) fn set_shell_bounds(
    window: WebviewWindow,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    form: String,
) -> Result<(), String> {
    let form = parse_form(&form)?;
    let spec = form_spec(form);
    apply_chrome(&window, &spec)?;
    apply_min(&window, min_size(form, width, height))?;
    window
        .set_size(LogicalSize::new(width, height))
        .map_err(|err| err.to_string())?;
    window
        .set_position(LogicalPosition::new(x, y))
        .map_err(|err| err.to_string())?;
    Ok(())
}

#[tauri::command]
pub(crate) fn window_minimize(window: WebviewWindow) -> Result<(), String> {
    window.minimize().map_err(|err| err.to_string())
}

#[tauri::command]
pub(crate) fn window_toggle_maximize(window: WebviewWindow) -> Result<(), String> {
    if window.is_maximized().map_err(|err| err.to_string())? {
        window.unmaximize().map_err(|err| err.to_string())
    } else {
        window.maximize().map_err(|err| err.to_string())
    }
}

#[tauri::command]
pub(crate) fn window_close(window: WebviewWindow) -> Result<(), String> {
    window.close().map_err(|err| err.to_string())
}

/// Main-window chrome at launch: the full form.
pub(crate) fn apply_full_chrome(window: &WebviewWindow) {
    let _ = apply_chrome(window, &form_spec(ShellForm::Full));
    let _ = apply_min(window, Some((FULL_MIN_WIDTH, FULL_MIN_HEIGHT)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_settled_full_form_has_a_minimum() {
        assert_eq!(
            min_size(ShellForm::Full, 1360.0, 828.0),
            Some((FULL_MIN_WIDTH, FULL_MIN_HEIGHT))
        );
        assert_eq!(min_size(ShellForm::Full, 520.0, 600.0), None);
        assert_eq!(min_size(ShellForm::Companion, 400.0, 790.0), None);
        assert_eq!(min_size(ShellForm::Pill, 400.0, 52.0), None);
        assert!(FULL_MIN_WIDTH >= 700.0);
    }
}
