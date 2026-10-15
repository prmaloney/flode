//! The pinned baseview uses raw-window-handle 0.5, while iced and nih-plug use 0.4. These convert
//! between the two. Only AppKit handles are handled, which is all the macOS build needs.

use raw_window_handle::RawWindowHandle as Handle04;
use raw_window_handle_05::{AppKitWindowHandle, HasRawWindowHandle as HasHandle05, RawWindowHandle as Handle05};

/// Converts a 0.4 handle to a 0.5 handle.
pub(crate) fn to_05(handle: Handle04) -> Handle05 {
    match handle {
        Handle04::AppKit(h) => {
            let mut out = AppKitWindowHandle::empty();
            out.ns_window = h.ns_window;
            out.ns_view = h.ns_view;
            Handle05::AppKit(out)
        }
        _ => panic!("iced_baseview: only AppKit parent windows are supported on macOS"),
    }
}

/// Converts a 0.5 handle to a 0.4 handle.
pub(crate) fn to_04(handle: Handle05) -> Handle04 {
    match handle {
        Handle05::AppKit(h) => {
            let mut out = raw_window_handle::AppKitHandle::empty();
            out.ns_window = h.ns_window;
            out.ns_view = h.ns_view;
            Handle04::AppKit(out)
        }
        _ => panic!("iced_baseview: only AppKit windows are supported on macOS"),
    }
}

/// Wraps a 0.4 handle so baseview can take it as a 0.5 handle.
pub(crate) struct Adapter(pub Handle05);

unsafe impl HasHandle05 for Adapter {
    fn raw_window_handle(&self) -> Handle05 {
        self.0
    }
}
