#![cfg(unix)]

use std::{collections::BTreeSet, path::Path, process::Command};

#[test]
fn native_archive_exports_only_numeric_parsers() {
    let archive = Path::new(env!("OUT_DIR")).join("libpg_internals_guc_parse.a");
    let output = Command::new("nm")
        .args(["-g", "-P"])
        .arg(archive)
        .output()
        .expect("nm is required to inspect the native parser archive");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let symbols = String::from_utf8(output.stdout).expect("non-UTF-8 symbol table");
    let definitions: BTreeSet<_> = symbols
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let name = fields.next()?;
            let kind = fields.next()?;
            // Portable nm format: name, type, value, size. Ignore member headings
            // and undefined libc/libm symbols; Mach-O prefixes C names with '_'.
            (kind.len() == 1 && kind != "U").then(|| name.trim_start_matches('_'))
        })
        .collect();
    assert_eq!(
        definitions,
        BTreeSet::from(["pg_internals_parse_int", "pg_internals_parse_real"])
    );
}
