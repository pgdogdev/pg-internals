use std::{
    env,
    path::{Path, PathBuf},
};

/// Compile PostgreSQL's GUC parsers; the linker discards unused backend code.
pub fn build(source: &Path) {
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("missing output directory"));
    let wrapper =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("missing crate directory"))
            .join("build");
    for path in [
        wrapper.join("guc_parse.c"),
        wrapper.join("guc_parse.h"),
        source.join("src/backend/utils/misc/guc.c"),
    ] {
        println!("cargo:rerun-if-changed={}", path.display());
    }

    cc::Build::new()
        .file(wrapper.join("guc_parse.c"))
        .extra_warnings(false)
        .include(output.join("postgres/src/include"))
        .include(source.join("src/include"))
        .include(source.join("src/backend"))
        .define("_GNU_SOURCE", None)
        // Keep the parsers independently linkable from guc.c's backend routines.
        .flag_if_supported("-ffunction-sections")
        .flag_if_supported("-fdata-sections")
        .compile("pg_internals_guc_parse");
    if env::var("CARGO_CFG_TARGET_FAMILY").is_ok_and(|family| family == "unix") {
        println!("cargo:rustc-link-lib=m");
    }
}
