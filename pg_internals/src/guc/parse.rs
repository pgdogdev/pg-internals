use std::{
    ffi::{CString, c_char, c_double, c_int},
    ptr,
};

use crate::constants::GUC_UNIT;

unsafe extern "C" {
    fn pg_internals_parse_int(
        value: *const c_char,
        result: *mut c_int,
        flags: c_int,
        hint: *mut *const c_char,
    ) -> bool;
    fn pg_internals_parse_real(
        value: *const c_char,
        result: *mut c_double,
        flags: c_int,
        hint: *mut *const c_char,
    ) -> bool;
}

/// Parse a PostgreSQL integer setting, converting units and rounding as PostgreSQL does.
///
/// `flags` contains the setting's GUC_UNIT_* base unit, or zero for no units.
/// Decimal, octal, hexadecimal, and fractional inputs are supported. Returns None
/// for invalid input, integer overflow, or an interior NUL. Setting-specific
/// bounds and check hooks are not applied. Uses the process's C locale and
/// floating-point rounding mode, just like PostgreSQL's parser.
pub fn parse_int(value: &str, flags: u32) -> Option<i32> {
    let value = CString::new(value).ok()?;
    let mut result = 0;
    // SAFETY: value is NUL-terminated and live for the call, result is writable,
    // and PostgreSQL permits a NULL hint pointer. No pointer is retained.
    let valid = unsafe {
        pg_internals_parse_int(
            value.as_ptr(),
            &mut result,
            (flags & GUC_UNIT) as c_int,
            ptr::null_mut(),
        )
    };
    valid.then_some(result)
}

/// Parse a PostgreSQL real setting and convert it to the base unit in `flags`.
///
/// Returns None for invalid input, NaN, strtod range errors, or an interior NUL.
/// Like PostgreSQL, this may return infinity; callers must check setting bounds.
/// Uses the process's C locale and floating-point rounding mode.
pub fn parse_real(value: &str, flags: u32) -> Option<f64> {
    let value = CString::new(value).ok()?;
    let mut result = 0.0;
    // SAFETY: value and result are valid for the duration of the call. PostgreSQL
    // permits a NULL hint pointer and does not retain any of these pointers.
    let valid = unsafe {
        pg_internals_parse_real(
            value.as_ptr(),
            &mut result,
            (flags & GUC_UNIT) as c_int,
            ptr::null_mut(),
        )
    };
    valid.then_some(result)
}
