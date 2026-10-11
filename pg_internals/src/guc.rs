//! Build-time metadata for GUCs with compile-time defaults and bounds.
//!
//! Integer arithmetic is evaluated by Rust at compile time. Conditional entries,
//! unresolved constants, and unsupported C syntax are omitted. Names use
//! PostgreSQL's original spelling and lookups are case-sensitive.

/// A PostgreSQL configuration value resolved at compile time.
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
    pub min: Option<Value>,
    pub max: Option<Value>,
}

include!(concat!(env!("OUT_DIR"), "/guc.rs"));
