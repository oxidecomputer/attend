//! macOS accessibility backend for external text capture.
//!
//! Uses the macOS Accessibility API to query the focused application for
//! selected text. The frontmost app is identified via `NSWorkspace` (AppKit),
//! then its accessibility tree is queried via `AXUIElement` for the focused
//! element and selected text.
//!
//! On macOS Sequoia, the system-wide AX element's `AXFocusedApplication`
//! attribute fails for unsigned binaries. We work around this by getting
//! the frontmost app PID from `NSWorkspace.shared.frontmostApplication`,
//! which doesn't require accessibility permissions, then constructing
//! an `AXUIElement` from the PID.

use accessibility::AXUIElement;
use accessibility_sys::AXUIElementCopyAttributeValue;
use core_foundation::base::{CFType, TCFType};
use core_foundation::string::CFString;

use super::{ExternalSnapshot, ExternalSource};

/// macOS accessibility source.
///
/// Queries the frontmost application for selected text. Requires Accessibility
/// permission for the calling process (or its responsible process, typically
/// the terminal emulator).
pub struct MacOsSource;

impl MacOsSource {
    pub fn new() -> Self {
        Self
    }
}

/// Information about the frontmost application, obtained via NSWorkspace.
struct FrontmostApp {
    name: String,
    pid: i32,
}

/// Get the frontmost application's name and PID via NSWorkspace.
///
/// Uses Objective-C message sends via the `objc` crate (already a transitive
/// dependency). This does not require accessibility permissions.
fn frontmost_app() -> Option<FrontmostApp> {
    use objc::msg_send;
    use objc::runtime::{Class, Object};

    unsafe {
        let workspace_class = Class::get("NSWorkspace")?;
        let shared: *mut Object = msg_send![workspace_class, sharedWorkspace];
        if shared.is_null() {
            return None;
        }

        let app: *mut Object = msg_send![shared, frontmostApplication];
        if app.is_null() {
            return None;
        }

        let pid: i32 = msg_send![app, processIdentifier];

        // Get localized name (NSString) and convert to Rust String.
        let ns_name: *mut Object = msg_send![app, localizedName];
        let name = if ns_name.is_null() {
            String::new()
        } else {
            let utf8: *const std::ffi::c_char = msg_send![ns_name, UTF8String];
            if utf8.is_null() {
                String::new()
            } else {
                std::ffi::CStr::from_ptr(utf8)
                    .to_string_lossy()
                    .into_owned()
            }
        };

        Some(FrontmostApp { name, pid })
    }
}

/// Query a raw AX attribute that returns a CFType.
///
/// Returns `None` if the attribute doesn't exist, has no value, or the
/// query fails for any reason.
fn ax_raw_attr(element: &AXUIElement, attr_name: &str) -> Option<CFType> {
    unsafe {
        let attr_cf = CFString::new(attr_name);
        let mut value: core_foundation::base::CFTypeRef = std::ptr::null();
        let err = AXUIElementCopyAttributeValue(
            element.as_concrete_TypeRef(),
            attr_cf.as_concrete_TypeRef(),
            &mut value,
        );
        if err == 0 && !value.is_null() {
            Some(TCFType::wrap_under_create_rule(value))
        } else {
            None
        }
    }
}

/// Extract a string value from a CFType, if it is a CFString.
fn cf_as_string(cf: &CFType) -> Option<String> {
    if cf.instance_of::<CFString>() {
        let s: CFString = unsafe { TCFType::wrap_under_get_rule(cf.as_CFTypeRef() as *const _) };
        Some(s.to_string())
    } else {
        None
    }
}

/// Extract an AXUIElement from a CFType, if it is one.
fn cf_as_element(cf: CFType) -> Option<AXUIElement> {
    if cf.type_of() == AXUIElement::type_id() {
        Some(unsafe { TCFType::wrap_under_get_rule(cf.as_CFTypeRef() as _) })
    } else {
        None
    }
}

impl ExternalSource for MacOsSource {
    fn is_available(&self) -> bool {
        // Probe: get the frontmost app via NSWorkspace and try an AX query.
        // If accessibility permission is not granted, the AX query will fail.
        if let Some(front) = frontmost_app() {
            let app = AXUIElement::application(front.pid);
            ax_raw_attr(&app, "AXTitle").is_some()
        } else {
            false
        }
    }

    fn query(&self) -> Option<ExternalSnapshot> {
        // Step 1: Get the frontmost app via NSWorkspace (no AX needed).
        let front = frontmost_app()?;
        let app_element = AXUIElement::application(front.pid);

        // Step 2: Get the focused window title.
        let window_title = ax_raw_attr(&app_element, "AXFocusedWindow")
            .and_then(cf_as_element)
            .and_then(|w| ax_raw_attr(&w, "AXTitle"))
            .and_then(|v| cf_as_string(&v))
            .unwrap_or_default();

        // Step 3: Get the focused UI element and its selected text.
        let focused = ax_raw_attr(&app_element, "AXFocusedUIElement").and_then(cf_as_element);
        let selected_text = focused
            .and_then(|el| ax_raw_attr(&el, "AXSelectedText"))
            .and_then(|v| cf_as_string(&v));

        Some(ExternalSnapshot {
            app: front.name,
            window_title,
            selected_text: selected_text.filter(|t| !t.is_empty()),
        })
    }
}
