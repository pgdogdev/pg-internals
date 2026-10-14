use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

/// Generate PostgreSQL's configuration headers using its default build options.
pub fn build(source: &Path) {
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("postgres");

    for path in [
        "configure",
        "config",
        "GNUmakefile.in",
        "src/Makefile.global.in",
        "src/include/pg_config.h.in",
        "src/include/port",
        "src/template",
        "src/interfaces/ecpg/include/ecpg_config.h.in",
    ] {
        println!("cargo:rerun-if-changed={}", source.join(path).display());
    }
    for variable in [
        "PATH",
        "CC",
        "CFLAGS",
        "CPP",
        "CPPFLAGS",
        "LDFLAGS",
        "LIBS",
        "PKG_CONFIG",
        "PKG_CONFIG_PATH",
        "PKG_CONFIG_LIBDIR",
        "ICU_CFLAGS",
        "ICU_LIBS",
        "CONFIG_SITE",
    ] {
        println!("cargo:rerun-if-env-changed={variable}");
    }

    run(source, &output);
}

/// Configure in a separate build directory, also used by the vendoring command.
pub fn run(source: &Path, output: &Path) {
    // Older builds generated backend headers here. They must not shadow the
    // vendored headers, including after changing the pinned PostgreSQL revision.
    if output.exists() {
        fs::remove_dir_all(output).expect("cannot clear the PostgreSQL build directory");
    }
    fs::create_dir_all(output).expect("cannot create the PostgreSQL build directory");
    let mut configure = Command::new(source.join("configure"));
    configure.current_dir(output);
    #[cfg(target_os = "macos")]
    homebrew_icu(&mut configure);
    let configured = configure.output().expect("cannot run PostgreSQL configure");
    let log = output.join("configure.log");
    let mut transcript = configured.stdout;
    transcript.extend_from_slice(&configured.stderr);
    fs::write(&log, &transcript).expect("cannot write the PostgreSQL configure log");
    assert!(
        configured.status.success(),
        "PostgreSQL configure failed ({}). See {} and {}.\n{}",
        configured.status,
        log.display(),
        output.join("config.log").display(),
        String::from_utf8_lossy(&configured.stderr)
    );
}

#[cfg(target_os = "macos")]
fn homebrew_icu(configure: &mut Command) {
    // An explicit library search root may belong to a different target/sysroot.
    if env::var_os("PKG_CONFIG_LIBDIR").is_some() {
        return;
    }
    let Ok(prefix) = Command::new("brew").arg("--prefix").output() else {
        return;
    };
    if !prefix.status.success() {
        return;
    }
    let Ok(prefix) = std::str::from_utf8(&prefix.stdout) else {
        return;
    };
    let Ok(installed) = fs::read_dir(Path::new(prefix.trim()).join("opt")) else {
        return;
    };

    // Inspect installed kegs: `brew --prefix icu4c` can name an uninstalled version.
    let mut icu_paths: Vec<_> = installed
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name == "icu4c" || name.starts_with("icu4c@")
        })
        .map(|entry| entry.path().join("lib/pkgconfig"))
        .filter(|path| path.join("icu-uc.pc").is_file() && path.join("icu-i18n.pc").is_file())
        .collect();
    if icu_paths.is_empty() {
        return;
    }
    icu_paths.sort();
    let mut paths: Vec<_> = env::var_os("PKG_CONFIG_PATH")
        .map(|value| env::split_paths(&value).collect())
        .unwrap_or_default();
    paths.extend(icu_paths);
    configure.env(
        "PKG_CONFIG_PATH",
        env::join_paths(paths).expect("invalid pkg-config search path"),
    );
}
