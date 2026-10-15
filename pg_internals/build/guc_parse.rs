use std::{
    env, fs,
    path::{Path, PathBuf},
};

/// Compile only PostgreSQL's numeric GUC parsers and their private dependencies.
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

    let upstream = fs::read_to_string(source.join("src/backend/utils/misc/guc.c"))
        .expect("cannot read PostgreSQL GUC source");
    let mut parsers =
        fs::read_to_string(wrapper.join("guc_parse.c")).expect("cannot read GUC parser includes");
    // Keep the pinned upstream implementation verbatim, but never compile the
    // backend routines: even discarded sections can cause TLS symbol conflicts
    // with libpg_query before the linker performs garbage collection.
    for (declaration, end) in [
        ("#define MAX_UNIT_LEN", "\n};"),
        ("static const char *const time_units_hint", "\n};"),
        ("static bool\nconvert_to_base_unit(", "\n}"),
        ("bool\nparse_int(", "\n}"),
        ("bool\nparse_real(", "\n}"),
    ] {
        parsers.push_str(extract(&upstream, declaration, end));
        parsers.push('\n');
    }
    let generated = output.join("guc_parse.c");
    fs::write(&generated, parsers).expect("cannot write isolated GUC parsers");

    cc::Build::new()
        .file(generated)
        .include(&wrapper)
        .extra_warnings(false)
        .include(output.join("postgres/src/include"))
        .include(source.join("src/include"))
        .include(source.join("src/backend"))
        .define("_GNU_SOURCE", None)
        .compile("pg_internals_guc_parse");
    if env::var("CARGO_CFG_TARGET_FAMILY").is_ok_and(|family| family == "unix") {
        println!("cargo:rustc-link-lib=m");
    }
}

/// PostgreSQL puts top-level closing braces at column zero. Fail the build if
/// the pinned source changes instead of silently falling back to all of guc.c.
fn extract<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    let mut matches = source.match_indices(start);
    let (offset, _) = matches
        .next()
        .unwrap_or_else(|| panic!("missing GUC declaration {start}"));
    assert!(
        matches.next().is_none(),
        "ambiguous GUC declaration {start}"
    );
    let source = &source[offset..];
    let (offset, _) = source
        .match_indices(end)
        .next()
        .unwrap_or_else(|| panic!("missing end of GUC declaration {start}"));
    &source[..offset + end.len()]
}
