//! Build-time metadata for GUCs with compile-time defaults and bounds.
//!
//! Integer arithmetic is evaluated by Rust at compile time. Conditional entries,
//! unresolved constants, and unsupported C syntax are omitted. Names use
//! PostgreSQL's original spelling and lookups are case-sensitive.

mod datestyle;
mod parse;
pub use datestyle::{DateStyleError, DateStyleInput, normalize_datestyle};
pub use parse::{parse_int, parse_real};

/// A PostgreSQL configuration value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i32),
    Real(f64),
    String(&'static str),
    /// A null string default, distinct from an empty string.
    Null,
    /// PostgreSQL's numeric enum value, not its user-facing option name.
    Enum(i32),
}

const fn integer_constant(value: i128) -> i32 {
    assert!(value >= i32::MIN as i128 && value <= i32::MAX as i128);
    value as i32
}

const fn string_constant(bytes: &'static [u8]) -> &'static str {
    assert!(!bytes.is_empty() && bytes[bytes.len() - 1] == 0);
    let (text, _) = bytes.split_at(bytes.len() - 1);
    match std::str::from_utf8(text) {
        Ok(text) => text,
        Err(_) => panic!("PostgreSQL string constant is not UTF-8"),
    }
}

/// Configuration metadata extracted from PostgreSQL's `guc_parameters.dat`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Guc {
    pub name: &'static str,
    pub context: &'static str,
    pub group: &'static str,
    pub short_desc: &'static str,
    pub long_desc: Option<&'static str>,
    pub boot_val: Value,
    /// PostgreSQL GUC_UNIT_* base-unit flag, or zero for a unitless setting.
    pub unit_flags: u32,
    /// Whether PostgreSQL reports this setting via ParameterStatus (GUC_REPORT).
    pub report: bool,
    pub min: Option<Value>,
    pub max: Option<Value>,
    /// Accepted enum strings, including hidden aliases, for the configured build.
    /// Empty for settings that are not enums.
    pub options: &'static [&'static str],
}

include!(concat!(env!("OUT_DIR"), "/guc.rs"));

impl Guc {
    /// Parse a numeric value into this setting's base units using PostgreSQL.
    ///
    /// Returns None for malformed values, integer overflow, interior NUL bytes,
    /// or a nonnumeric setting. This does not check min/max or backend hooks.
    /// Real infinities can parse successfully and must be checked against bounds.
    pub fn parse_numeric(&self, value: &str) -> Option<Value> {
        match self.boot_val {
            Value::Int(_) => parse_int(value, self.unit_flags).map(Value::Int),
            Value::Real(_) => parse_real(value, self.unit_flags).map(Value::Real),
            _ => None,
        }
    }
}
