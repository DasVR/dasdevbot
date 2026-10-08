//! Windows 11 Snap Layouts on the drawn maximize button (CD ruling a).
//!
//! Windows shows the Snap Layouts flyout only over a window that answers
//! `WM_NCHITTEST` with `HTMAXBUTTON`. With `decorations: false` the main
//! window's client area is covered by WebView2's own child window, which
//! answers the hit-test first, so a subclass of the main window never sees the
//! cursor over the drawn button. Instead a small native child window sits
//! exactly over the drawn maximize button and returns `HTMAXBUTTON` for every
//! point. It never paints, so the drawn button shows through.
//!
//! The child owns the mouse in that rectangle, so the drawn button gets no DOM
//! hover or click there. The child reports `hover`, `press`, `release` and
//! `leave` to the page as a `dasdevbot:snap-maximize` CustomEvent (the same
//! eval path as native_sight.rs), and toggles maximize itself on a click. The page sends the button's rectangle (CSS px) whenever it moves
//! and `None` when the captions are not shown.
//!
//! Windows-only. Other targets keep the command as a no-op so the page can call
//! it unconditionally. Unverified on real Windows until Arriq's build.

use serde::Deserialize;
use tauri::WebviewWindow;

/// The drawn maximize button, in CSS px from the window's top-left.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) struct SnapRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// The DOM event the overlay dispatches in the page.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) const SNAP_EVENT: &str = "dasdevbot:snap-maximize";

/// The script that tells the page what the pointer did on the overlay.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn snap_script(state: &str) -> String {
    format!("window.dispatchEvent(new CustomEvent({SNAP_EVENT:?},{{detail:{{state:{state:?}}}}}))")
}

/// Physical pixels for a CSS-px rectangle at `scale`. Edges round outward so
/// the overlay never leaves a sliver of the drawn button uncovered.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn physical(rect: SnapRect, scale: f64) -> (i32, i32, i32, i32) {
    let left = (rect.x * scale).floor();
    let top = (rect.y * scale).floor();
    let right = ((rect.x + rect.width) * scale).ceil();
    let bottom = ((rect.y + rect.height) * scale).ceil();
    (
        left as i32,
        top as i32,
        (right - left).max(0.0) as i32,
        (bottom - top).max(0.0) as i32,
    )
}

#[tauri::command]
pub(crate) fn snap_maximize_rect(window: WebviewWindow, rect: Option<SnapRect>) -> Result<(), String> {
    #[cfg(windows)]
    {
        let scale = window.scale_factor().map_err(|err| err.to_string())?;
        let target = window.clone();
        window
            .run_on_main_thread(move || imp::place(&target, rect.map(|r| physical(r, scale))))
            .map_err(|err| err.to_string())?;
    }
    #[cfg(not(windows))]
    {
        let _ = (window, rect);
    }
    Ok(())
}

#[cfg(windows)]
mod imp {
    use std::cell::{Cell, OnceCell};

    use tauri::WebviewWindow;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::{GetStockObject, ValidateRect, HBRUSH, NULL_BRUSH};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        TrackMouseEvent, TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, LoadCursorW, RegisterClassW, SetWindowPos, ShowWindow,
        HTMAXBUTTON, HWND_TOP, IDC_ARROW, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, WM_ERASEBKGND,
        WM_NCHITTEST, WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP, WM_NCMOUSELEAVE,
        WM_NCMOUSEMOVE, WM_PAINT, WNDCLASSW, WS_CHILD, WS_CLIPSIBLINGS, WS_VISIBLE,
    };

    use super::snap_script;

    // Everything here runs on the UI thread: the command hops there and the
    // window procedure is called there. Cells, not a mutex, because Win32 calls
    // re-enter the procedure synchronously.
    thread_local! {
        static OVERLAY: Cell<HWND> = const { Cell::new(std::ptr::null_mut()) };
        static HOVER: Cell<bool> = const { Cell::new(false) };
        static PRESSED: Cell<bool> = const { Cell::new(false) };
        static WINDOW: OnceCell<WebviewWindow> = const { OnceCell::new() };
        static CLASS: Cell<bool> = const { Cell::new(false) };
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn class_name() -> Vec<u16> {
        wide("DasdevbotSnapMaximize")
    }

    fn emit(what: &str) {
        let window = WINDOW.with(|cell| cell.get().cloned());
        if let Some(window) = window {
            let _ = window.eval(snap_script(what));
        }
    }

    fn toggle() {
        let window = WINDOW.with(|cell| cell.get().cloned());
        if let Some(window) = window {
            let _ = match window.is_maximized() {
                Ok(true) => window.unmaximize(),
                _ => window.maximize(),
            };
        }
    }

    /// The overlay's window procedure. Every point is the maximize button.
    pub(super) unsafe extern "system" fn procedure(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_NCHITTEST => HTMAXBUTTON as LRESULT,
            WM_NCMOUSEMOVE => {
                if !HOVER.with(|hover| hover.replace(true)) {
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE | TME_NONCLIENT,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    TrackMouseEvent(&mut track);
                    emit("hover");
                }
                // The system's own caption-button hover tracking drives the flyout.
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            WM_NCMOUSELEAVE => {
                HOVER.with(|hover| hover.set(false));
                PRESSED.with(|pressed| pressed.set(false));
                emit("leave");
                0
            }
            // No DefWindowProc: on HTMAXBUTTON it would start its own modal
            // caption-button loop on a child that has no caption.
            WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK => {
                PRESSED.with(|pressed| pressed.set(true));
                emit("press");
                0
            }
            WM_NCLBUTTONUP => {
                if PRESSED.with(|pressed| pressed.replace(false)) {
                    emit("release");
                    toggle();
                }
                0
            }
            // Never paint: the drawn button shows through.
            WM_PAINT => {
                ValidateRect(hwnd, std::ptr::null());
                0
            }
            WM_ERASEBKGND => 1,
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }

    fn register() -> bool {
        if CLASS.with(Cell::get) {
            return true;
        }
        let name = class_name();
        // SAFETY: plain Win32 class registration with a static procedure; the
        // name buffer outlives the call and the system copies it.
        let atom = unsafe {
            let class = WNDCLASSW {
                style: 0,
                lpfnWndProc: Some(procedure),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: GetModuleHandleW(std::ptr::null()),
                hIcon: std::ptr::null_mut(),
                hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
                hbrBackground: GetStockObject(NULL_BRUSH) as HBRUSH,
                lpszMenuName: std::ptr::null(),
                lpszClassName: name.as_ptr(),
            };
            RegisterClassW(&class)
        };
        CLASS.with(|class| class.set(atom != 0));
        atom != 0
    }

    fn overlay(window: &WebviewWindow) -> Option<HWND> {
        let existing = OVERLAY.with(Cell::get);
        if !existing.is_null() {
            return Some(existing);
        }
        if !register() {
            return None;
        }
        let parent = window.hwnd().ok()?.0 as HWND;
        let name = class_name();
        // SAFETY: a child of the main window, created on the UI thread that
        // owns the parent. No extended styles: layering would cost the hit-test.
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                name.as_ptr(),
                std::ptr::null(),
                WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
                0,
                0,
                0,
                0,
                parent,
                std::ptr::null_mut(),
                GetModuleHandleW(std::ptr::null()),
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            return None;
        }
        WINDOW.with(|cell| {
            let _ = cell.set(window.clone());
        });
        OVERLAY.with(|cell| cell.set(hwnd));
        Some(hwnd)
    }

    pub(super) fn place(window: &WebviewWindow, rect: Option<(i32, i32, i32, i32)>) {
        match rect {
            Some((x, y, width, height)) if width > 0 && height > 0 => {
                let Some(hwnd) = overlay(window) else {
                    return;
                };
                // SAFETY: our own child window. HWND_TOP keeps it above the
                // WebView2 sibling each time the page reports a new position.
                unsafe {
                    SetWindowPos(hwnd, HWND_TOP, x, y, width, height, SWP_NOACTIVATE | SWP_SHOWWINDOW);
                }
            }
            _ => {
                let hwnd = OVERLAY.with(Cell::get);
                if !hwnd.is_null() {
                    // SAFETY: our own child window.
                    unsafe {
                        ShowWindow(hwnd, SW_HIDE);
                    }
                    HOVER.with(|hover| hover.set(false));
                    PRESSED.with(|pressed| pressed.set(false));
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn every_point_of_the_overlay_is_the_maximize_button() {
            // SAFETY: WM_NCHITTEST is answered before the handle is used.
            let hit = unsafe { procedure(std::ptr::null_mut(), WM_NCHITTEST, 0, 0) };
            assert_eq!(hit, HTMAXBUTTON as LRESULT);
        }

        #[test]
        fn the_overlay_class_registers() {
            assert!(register());
            assert!(register(), "a second registration is a no-op");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rect_scales_and_rounds_outward() {
        let rect = SnapRect {
            x: 1268.0,
            y: 0.0,
            width: 46.0,
            height: 46.0,
        };
        assert_eq!(physical(rect, 1.0), (1268, 0, 46, 46));
        assert_eq!(physical(rect, 1.5), (1902, 0, 69, 69));
        let odd = SnapRect {
            x: 10.3,
            y: 0.4,
            width: 46.0,
            height: 32.0,
        };
        assert_eq!(physical(odd, 1.25), (12, 0, 59, 41));
    }

    #[test]
    fn the_page_hears_a_dom_event() {
        assert_eq!(
            snap_script("hover"),
            r#"window.dispatchEvent(new CustomEvent("dasdevbot:snap-maximize",{detail:{state:"hover"}}))"#
        );
    }
}
