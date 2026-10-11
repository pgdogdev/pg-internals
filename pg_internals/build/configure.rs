use std::{env, fs, path::PathBuf, process::Command};

/// Generate PostgreSQL's configuration headers using its default build options.
pub fn build() {
    let source = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("../postgres")
        .canonicalize()
        .expect("cannot locate the PostgreSQL source tree");
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
        "CONFIG_SITE",
    ] {
        println!("cargo:rerun-if-env-changed={variable}");
    }

    fs::create_dir_all(&output).expect("cannot create the PostgreSQL build directory");
    let configured = Command::new(source.join("configure"))
        .current_dir(&output)
        .output()
        .expect("cannot run PostgreSQL configure");
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
