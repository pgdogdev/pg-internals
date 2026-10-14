use std::{env, path::PathBuf};

/// Normal builds consume the checked-in subset and never fetch PostgreSQL.
pub fn prepare() -> PathBuf {
    let source = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("vendor/postgres");
    println!("cargo:rerun-if-changed={}", source.display());
    assert!(
        source.join("MANIFEST.json").is_file(),
        "missing vendored PostgreSQL source; run: cargo run --manifest-path xtask/Cargo.toml -- vendor-postgres"
    );
    source
        .canonicalize()
        .expect("cannot locate vendored PostgreSQL")
}
