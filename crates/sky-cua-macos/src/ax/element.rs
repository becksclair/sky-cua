//! Thin, typed wrappers over the AXUIElement C API.
//!
//! Every method here is a blocking IPC round trip into the target
//! application (except where noted) and must only be called on an `ax` lane
//! thread.

use std::ffi::c_void;
use std::ptr::{self, NonNull};

use objc2_application_services::{
    AXCopyMultipleAttributeOptions, AXError, AXUIElement, AXValue, AXValueType,
};
use objc2_core_foundation::{
    CFArray, CFBoolean, CFNumber, CFRange, CFRetained, CFString, CFType, CGPoint, CGSize,
};

/// An AX error code as returned by the C API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AxErr(pub i32);

impl AxErr {
    pub(crate) const FAILURE: Self = Self(AXError::Failure.0);
    pub(crate) const INVALID_ELEMENT: Self = Self(AXError::InvalidUIElement.0);
    pub(crate) const CANNOT_COMPLETE: Self = Self(AXError::CannotComplete.0);
    pub(crate) const ATTRIBUTE_UNSUPPORTED: Self = Self(AXError::AttributeUnsupported.0);
    pub(crate) const ACTION_UNSUPPORTED: Self = Self(AXError::ActionUnsupported.0);
    pub(crate) const NOT_IMPLEMENTED: Self = Self(AXError::NotImplemented.0);
    pub(crate) const API_DISABLED: Self = Self(AXError::APIDisabled.0);
    pub(crate) const NO_VALUE: Self = Self(AXError::NoValue.0);

    /// The messaging timeout expired or the target stopped answering.
    pub(crate) fn is_timeout(self) -> bool {
        self == Self::CANNOT_COMPLETE
    }

    /// The element or attribute simply does not support the request.
    pub(crate) fn is_unsupported(self) -> bool {
        matches!(
            self,
            Self::ATTRIBUTE_UNSUPPORTED
                | Self::ACTION_UNSUPPORTED
                | Self::NOT_IMPLEMENTED
                | Self::NO_VALUE
        )
    }

    pub(crate) fn name(self) -> &'static str {
        match self.0 {
            0 => "kAXErrorSuccess",
            -25200 => "kAXErrorFailure",
            -25201 => "kAXErrorIllegalArgument",
            -25202 => "kAXErrorInvalidUIElement",
            -25204 => "kAXErrorCannotComplete",
            -25205 => "kAXErrorAttributeUnsupported",
            -25206 => "kAXErrorActionUnsupported",
            -25208 => "kAXErrorNotImplemented",
            -25211 => "kAXErrorAPIDisabled",
            -25212 => "kAXErrorNoValue",
            -25213 => "kAXErrorParameterizedAttributeUnsupported",
            _ => "kAXError",
        }
    }
}

impl std::fmt::Display for AxErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.name(), self.0)
    }
}

fn check(error: AXError) -> Result<(), AxErr> {
    if error == AXError::Success {
        Ok(())
    } else {
        Err(AxErr(error.0))
    }
}

/// A retained `AXUIElementRef`.
#[derive(Clone)]
pub(crate) struct AxElement(CFRetained<AXUIElement>);

// SAFETY: an AXUIElementRef is an immutable CoreFoundation object (a process
// id plus an opaque element token). The AX client API is documented as usable
// from any thread and does not bind a reference to the thread that created
// it; retain/release are atomic.
unsafe impl Send for AxElement {}
// SAFETY: see `Send`; no method mutates the referenced object.
unsafe impl Sync for AxElement {}

impl std::fmt::Debug for AxElement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AxElement")
    }
}

impl PartialEq for AxElement {
    /// `CFEqual`: two references to the same accessibility object.
    fn eq(&self, other: &Self) -> bool {
        let left: &CFType = &self.0;
        let right: &CFType = &other.0;
        left == right
    }
}

impl AxElement {
    /// The application element for `pid`. Local; no IPC.
    pub(crate) fn application(pid: i32) -> Self {
        // SAFETY: AXUIElementCreateApplication accepts any pid and returns a
        // retained element.
        Self(unsafe { AXUIElement::new_application(pid) })
    }

    #[cfg(test)]
    pub(crate) fn from_retained(element: CFRetained<AXUIElement>) -> Self {
        Self(element)
    }

    /// The system-wide element. Local; no IPC.
    pub(crate) fn system_wide() -> Self {
        // SAFETY: takes no arguments and returns a retained element.
        Self(unsafe { AXUIElement::new_system_wide() })
    }

    /// Sets the per-message timeout for this reference (or, on the
    /// system-wide element, for every reference in this process that has no
    /// timeout of its own). Local; no IPC.
    pub(crate) fn set_messaging_timeout(&self, seconds: f32) -> Result<(), AxErr> {
        // SAFETY: plain value argument on a valid element.
        check(unsafe { self.0.set_messaging_timeout(seconds) })
    }

    pub(crate) fn copy_attribute(&self, attribute: &CFString) -> Result<CFRetained<CFType>, AxErr> {
        let mut value: *const CFType = ptr::null();
        // SAFETY: `value` is a valid out-pointer; on success it holds a +1
        // reference that we take ownership of.
        check(unsafe {
            self.0
                .copy_attribute_value(attribute, NonNull::from(&mut value))
        })?;
        retained(value).ok_or(AxErr::NO_VALUE)
    }

    /// Reads several attributes in one round trip. Each slot is `Ok(value)`
    /// or the per-attribute error.
    pub(crate) fn copy_multiple(
        &self,
        attributes: &CFArray<CFString>,
    ) -> Result<Vec<Result<CFRetained<CFType>, AxErr>>, AxErr> {
        let mut values: *const CFArray = ptr::null();
        // SAFETY: `attributes` is an array of CFStrings; `values` is a valid
        // out-pointer that receives a +1 array.
        check(unsafe {
            self.0.copy_multiple_attribute_values(
                attributes.as_opaque(),
                AXCopyMultipleAttributeOptions::empty(),
                NonNull::from(&mut values),
            )
        })?;
        let Some(array) = NonNull::new(values.cast_mut()) else {
            return Err(AxErr::NO_VALUE);
        };
        // SAFETY: a non-null +1 CFArray of CFTypes.
        let array: CFRetained<CFArray<CFType>> =
            unsafe { CFRetained::from_raw(array.cast::<CFArray<CFType>>()) };
        Ok(array.iter().map(decode_slot).collect())
    }

    pub(crate) fn action_names(&self) -> Result<Vec<String>, AxErr> {
        let mut names: *const CFArray = ptr::null();
        // SAFETY: valid out-pointer that receives a +1 array of CFStrings.
        check(unsafe { self.0.copy_action_names(NonNull::from(&mut names)) })?;
        let Some(array) = NonNull::new(names.cast_mut()) else {
            return Ok(Vec::new());
        };
        // SAFETY: a non-null +1 CFArray.
        let array: CFRetained<CFArray<CFType>> =
            unsafe { CFRetained::from_raw(array.cast::<CFArray<CFType>>()) };
        Ok(array.iter().filter_map(|name| as_string(&name)).collect())
    }

    pub(crate) fn is_settable(&self, attribute: &CFString) -> Result<bool, AxErr> {
        let mut settable: u8 = 0;
        // SAFETY: valid out-pointer.
        check(unsafe {
            self.0
                .is_attribute_settable(attribute, NonNull::from(&mut settable))
        })?;
        Ok(settable != 0)
    }

    pub(crate) fn set_attribute(&self, attribute: &CFString, value: &CFType) -> Result<(), AxErr> {
        // SAFETY: the value type is the caller's contract with the attribute.
        check(unsafe { self.0.set_attribute_value(attribute, value) })
    }

    pub(crate) fn perform(&self, action: &CFString) -> Result<(), AxErr> {
        // SAFETY: plain call on a valid element.
        check(unsafe { self.0.perform_action(action) })
    }

    pub(crate) fn set_bool(&self, attribute: &CFString, value: bool) -> Result<(), AxErr> {
        self.set_attribute(attribute, CFBoolean::new(value))
    }
}

fn retained(value: *const CFType) -> Option<CFRetained<CFType>> {
    // SAFETY: the AX copy functions return +1 references (or null).
    NonNull::new(value.cast_mut()).map(|value| unsafe { CFRetained::from_raw(value) })
}

/// A slot of an `AXUIElementCopyMultipleAttributeValues` result is either the
/// value or an `AXValue` of type `kAXValueAXErrorType`.
fn decode_slot(value: CFRetained<CFType>) -> Result<CFRetained<CFType>, AxErr> {
    if let Some(ax_value) = value.downcast_ref::<AXValue>() {
        // SAFETY: AXValueGetType has no preconditions.
        if unsafe { ax_value.r#type() } == AXValueType::AXError {
            let mut code: i32 = 0;
            // SAFETY: an AXError payload is an i32.
            let ok = unsafe {
                ax_value.value(
                    AXValueType::AXError,
                    NonNull::from(&mut code).cast::<c_void>(),
                )
            };
            return Err(if ok { AxErr(code) } else { AxErr::FAILURE });
        }
    }
    if value
        .downcast_ref::<objc2_core_foundation::CFNull>()
        .is_some()
    {
        return Err(AxErr::NO_VALUE);
    }
    Ok(value)
}

pub(crate) fn as_string(value: &CFType) -> Option<String> {
    value.downcast_ref::<CFString>().map(ToString::to_string)
}

pub(crate) fn as_bool(value: &CFType) -> Option<bool> {
    if let Some(boolean) = value.downcast_ref::<CFBoolean>() {
        return Some(boolean.as_bool());
    }
    value
        .downcast_ref::<CFNumber>()
        .and_then(CFNumber::as_i64)
        .map(|number| number != 0)
}

pub(crate) fn as_f64(value: &CFType) -> Option<f64> {
    value.downcast_ref::<CFNumber>().and_then(CFNumber::as_f64)
}

/// True when the value is a `CFNumber` (sliders, steppers, progress bars).
pub(crate) fn is_number(value: &CFType) -> bool {
    value.downcast_ref::<CFNumber>().is_some()
}

pub(crate) fn as_point(value: &CFType) -> Option<(f64, f64)> {
    let mut point = CGPoint { x: 0.0, y: 0.0 };
    decode_ax_value(value, AXValueType::CGPoint, &mut point).then_some((point.x, point.y))
}

pub(crate) fn as_size(value: &CFType) -> Option<(f64, f64)> {
    let mut size = CGSize {
        width: 0.0,
        height: 0.0,
    };
    decode_ax_value(value, AXValueType::CGSize, &mut size).then_some((size.width, size.height))
}

pub(crate) fn as_range(value: &CFType) -> Option<(isize, isize)> {
    let mut range = CFRange::new(0, 0);
    decode_ax_value(value, AXValueType::CFRange, &mut range)
        .then_some((range.location, range.length))
}

fn decode_ax_value<T>(value: &CFType, kind: AXValueType, out: &mut T) -> bool {
    let Some(ax_value) = value.downcast_ref::<AXValue>() else {
        return false;
    };
    // SAFETY: `out` is a valid, correctly sized destination for `kind`
    // (callers pair CGPoint/CGSize/CFRange with the matching type tag), and
    // AXValueGetValue refuses a mismatched tag.
    unsafe { ax_value.value(kind, NonNull::from(out).cast::<c_void>()) }
}

/// The elements of an `AXChildren`/`AXWindows`-style array value, capped.
pub(crate) fn as_elements(value: &CFType, cap: usize) -> Vec<AxElement> {
    as_elements_capped(value, cap).0
}

/// The AX elements of an array value, at most `cap` of them, and whether the
/// array held more than `cap` items.
pub(crate) fn as_elements_capped(value: &CFType, cap: usize) -> (Vec<AxElement>, bool) {
    let Some(array) = value.downcast_ref::<CFArray>() else {
        return (Vec::new(), false);
    };
    // SAFETY: CFArray<Opaque> and CFArray<CFType> have the same layout; every
    // element of a CF array is a CFType.
    let array: &CFArray<CFType> = unsafe { &*ptr::from_ref(array).cast::<CFArray<CFType>>() };
    let (items, dropped) = take_at_most(array.iter(), cap);
    let elements = items
        .into_iter()
        .filter_map(|item| item.downcast::<AXUIElement>().ok().map(AxElement))
        .collect();
    (elements, dropped)
}

/// The first `cap` items, and whether any item past them was dropped.
pub(crate) fn take_at_most<T>(items: impl IntoIterator<Item = T>, cap: usize) -> (Vec<T>, bool) {
    let mut items = items.into_iter();
    let taken: Vec<T> = items.by_ref().take(cap).collect();
    let dropped = items.next().is_some();
    (taken, dropped)
}

pub(crate) fn cf_string(value: &str) -> CFRetained<CFString> {
    CFString::from_str(value)
}

pub(crate) fn cf_number(value: f64) -> CFRetained<CFNumber> {
    CFNumber::new_f64(value)
}

pub(crate) fn cf_strings(values: &[&str]) -> CFRetained<CFArray<CFString>> {
    let strings: Vec<CFRetained<CFString>> = values.iter().map(|value| cf_string(value)).collect();
    CFArray::from_retained_objects(&strings)
}
