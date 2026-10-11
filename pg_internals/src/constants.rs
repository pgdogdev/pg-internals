//! Constants from PostgreSQL headers for the configured build and target.
//!
//! Includes named GUC defaults and bounds, selected configuration macros, and
//! GUC and WAL enums, using PostgreSQL's own `guc_tables.c` definitions.

#![allow(non_camel_case_types, non_upper_case_globals)]

include!(concat!(env!("OUT_DIR"), "/constants.rs"));
